#![cfg_attr(windows, windows_subsystem = "windows")]
//! Runs from the package's private temporary directory, never the installed
//! application. No GUI, storage startup, model credentials or audio are loaded.
use anyhow::{Context, Result, bail, ensure};
use nomifun_keyboard::{maintenance::{FileReplacement, MaintenanceGuard, ResumeIntent}, service};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
#[path = "../maintenance/windows.rs"]
mod windows;

struct Options {
    session: PathBuf,
    installed: PathBuf,
    payload: Option<PathBuf>,
    parent: u32,
    resume: ResumeIntent,
    no_resume: bool,
    uninstall: bool,
    defer_app_on_commit: bool,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut session = None;
        let mut installed = None;
        let mut payload = None;
        let mut parent = None;
        let mut resume = ResumeIntent::default();
        let mut no_resume = false;
        let mut uninstall = false;
        let mut defer_app_on_commit = false;
        let mut args = std::env::args_os().skip(1);
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("--session") => session = Some(PathBuf::from(args.next().context("missing session directory")?)),
                Some("--installed") => installed = Some(PathBuf::from(args.next().context("missing installed executable")?)),
                Some("--payload") => payload = Some(PathBuf::from(args.next().context("missing staged payload")?)),
                Some("--parent-pid") => parent = Some(args.next().context("missing parent PID")?.to_str().context("invalid parent PID")?.parse::<u32>()?),
                Some("--resume-app") => resume.desktop = true,
                Some("--resume-service") => resume.service = true,
                Some("--no-resume") => no_resume = true,
                Some("--uninstall") => uninstall = true,
                Some("--defer-app-on-commit") => defer_app_on_commit = true,
                _ => bail!("unsupported maintenance argument"),
            }
        }
        let options = Self {
            session: session.context("session directory is required")?,
            installed: installed.context("installed executable is required")?,
            payload,
            parent: parent.context("parent PID is required")?,
            resume,
            no_resume,
            uninstall,
            defer_app_on_commit,
        };
        ensure!(options.parent != 0 && options.parent != std::process::id(), "invalid parent PID");
        ensure!(options.session.is_absolute() && options.session.is_dir() && !options.session.is_symlink(), "invalid maintenance session directory");
        ensure!(options.installed.is_absolute(), "installed executable must be absolute");
        ensure!(options.uninstall || options.payload.is_some(), "staged payload is required");
        for name in ["status.ini", "command", "command.processing", "outcome.json", "session.lock"] {
            ensure!(!options.session.join(name).is_symlink(), "session control path is a symbolic link");
        }
        Ok(options)
    }
}

fn status(options: &Options, state: &str, detail: &str) -> Result<()> {
    let detail = detail.replace(['\r', '\n', '\0'], " ");
    let value = format!("[maintenance]\r\nstate={state}\r\ndetail={detail}\r\n");
    // NSIS ReadINIStr consumes Unicode Windows INI files. Encode all statuses
    // consistently, including Chinese I/O and stop-proof diagnostics.
    let mut bytes = vec![0xff, 0xfe];
    for character in value.encode_utf16() { bytes.extend_from_slice(&character.to_le_bytes()); }
    let pending = options.session.join("status.next");
    fs::write(&pending, bytes)?;
    #[cfg(windows)]
    windows::replace_file(&pending, &options.session.join("status.ini"))?;
    #[cfg(not(windows))]
    fs::rename(&pending, options.session.join("status.ini"))?;
    Ok(())
}

fn launch(installed: &Path, argument: Option<&str>) -> Result<Child> {
    ensure!(installed.is_file() && !installed.is_symlink(), "installed executable is unavailable for recovery");
    let mut command = Command::new(installed);
    if let Some(argument) = argument { command.arg(argument); }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().context("restart installed NomiFun")
}

fn wait_for_service(child: &mut Child) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let runtime = service::runtime_dir()?;
    loop {
        let descriptor = fs::read(runtime.join("owner.json")).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if descriptor.as_ref().and_then(|value| value["pid"].as_u64()) == Some(child.id() as u64)
            && ResumeIntent::capture(&runtime)?.service { return Ok(()); }
        if let Some(status) = child.try_wait()? {
            bail!("keyboard service exited before readiness was confirmed: {status}");
        }
        ensure!(Instant::now() < deadline, "keyboard service started but readiness was not confirmed");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn resume(options: &Options, intent: ResumeIntent) -> Vec<String> {
    if options.no_resume { return Vec::new(); }
    let mut errors = Vec::new();
    // A failed preparation can leave the original process alive. Restart only
    // participants proven absent instead of duplicating every observed process.
    let mut running = match service::runtime_dir().and_then(|dir| ResumeIntent::capture(&dir)) {
        Ok(running) => running,
        Err(error) => return vec![format!("cannot verify recovery participants: {error:#}")],
    };
    #[cfg(windows)]
    match service::runtime_dir().and_then(|dir| windows::installed_desktop_running(&options.installed, &dir)) {
        Ok(desktop) => running.desktop |= desktop,
        Err(error) => return vec![format!("cannot verify installed application recovery: {error:#}")],
    }
    if intent.service && !running.service {
        if let Err(error) = launch(&options.installed, Some("--keyboard-service")).and_then(|mut child| wait_for_service(&mut child)) {
            errors.push(format!("keyboard service: {error:#}"));
        }
    }
    if intent.desktop && !running.desktop {
        if let Err(error) = launch(&options.installed, None) { errors.push(format!("desktop: {error:#}")); }
    }
    errors
}

fn finish(options: &Options, state: &str, detail: &str, intent: ResumeIntent) -> Result<()> {
    let warnings = resume(options, intent);
    let detail = if warnings.is_empty() { detail.to_owned() } else { format!("{detail} Recovery warning: {}", warnings.join("; ")) };
    // Diagnostic-file failure must not suppress the final status or change a
    // committed installation into an apparent rollback.
    let _ = fs::write(options.session.join("outcome.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "state": state, "detail": detail, "resumeWarnings": warnings,
        "resumeDesktopRequested": intent.desktop && !options.no_resume,
        "resumeServiceRequested": intent.service && !options.no_resume,
    }))?);
    // Resume warnings never turn a successfully committed application back into
    // an uncommitted installation. The caller can display the actionable warning.
    status(options, state, &detail)
}

#[cfg(not(windows))]
struct Parent(u32);
#[cfg(not(windows))]
impl Parent {
    fn open(pid: u32) -> Result<Self> { Ok(Self(pid)) }
    fn alive(&self) -> bool {
        unsafe extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
        unsafe { kill(self.0 as i32, 0) == 0 }
    }
}
#[cfg(windows)]
use windows::Parent;

fn claim_command(session: &Path) -> Result<Option<PathBuf>> {
    let command = session.join("command");
    let processing = session.join("command.processing");
    ensure!(!processing.exists() && !processing.is_symlink(), "maintenance command is already being processed");
    match fs::rename(command, &processing) {
        Ok(()) => {
            ensure!(!processing.is_symlink(), "maintenance command is a symbolic link");
            Ok(Some(processing))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn read_claimed_command(path: &Path) -> Result<String> {
    let command = fs::read_to_string(path)?;
    fs::remove_file(path)?;
    Ok(command.trim().to_owned())
}

fn run(options: &Options) -> Result<()> {
    use fs2::FileExt;
    let session_lock = File::options().create(true).truncate(false).read(true).write(true).open(options.session.join("session.lock"))?;
    FileExt::try_lock_exclusive(&session_lock).context("maintenance session is already active")?;
    let parent = Parent::open(options.parent)?;
    ensure!(parent.alive(), "installer already exited");
    let runtime = service::runtime_dir()?;
    let mut intent = ResumeIntent::capture(&runtime)?;
    intent.desktop |= options.resume.desktop;
    intent.service |= options.resume.service;
    #[cfg(windows)]
    {
        // Legacy versions have no desktop lifetime lock. Inspect exact image
        // paths without launching their unsupported management command.
        intent.desktop |= windows::installed_desktop_running(&options.installed, &runtime)?;
    }
    #[cfg(windows)]
    let (login_enabled, login_warning) = match windows::owns_login_registration(&options.installed) {
        Ok(enabled) => (enabled, String::new()),
        Err(error) if options.uninstall => return Err(error),
        Err(error) => (false, format!("Startup preference could not be inspected; its registration was preserved: {error:#}")),
    };
    #[cfg(not(windows))]
    let (login_enabled, login_warning) = (false, String::new());
    let checked_runtime = runtime.clone();
    let mut participants_checked = false;
    let prepared = MaintenanceGuard::prepare_with_intent_checked_in(runtime, Duration::from_secs(30), &mut intent, |observed| {
        #[cfg(windows)]
        windows::validate_runtime_ownership(&options.installed, &checked_runtime, observed)?;
        #[cfg(not(windows))]
        let _ = (&checked_runtime, observed);
        participants_checked = true;
        Ok(())
    });
    let guard = match prepared {
        Ok(guard) => guard,
        Err(error) => return finish(options, "error", &format!("keyboard maintenance preparation failed: {error:#}"), if participants_checked { intent } else { ResumeIntent::default() }),
    };
    let mut guard = Some(guard);
    let mut replacement: Option<FileReplacement> = None;
    let mut removed = false;
    // All failures after preparation take the same rollback/recovery path,
    // including unexpected command I/O and failed status publication.
    let completion = (|| -> Result<bool> {
      status(options, "ready", "")?;
      loop {
        // Claim the old command atomically before reading it. A newly
        // published abort then stays at `command` and cannot be deleted by
        // removal of the command currently being processed.
        let command = if let Some(path) = claim_command(&options.session)? {
            Some(read_claimed_command(&path)?)
        } else if !parent.alive() {
            Some("abort".to_owned())
        } else {
            None
        };
        let Some(command) = command else {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        };
        match command.as_str() {
            "apply" => {
                ensure!(!options.uninstall && replacement.is_none(), "invalid repeated application replacement");
                let payload = options.payload.as_deref().context("missing application payload")?;
                replacement = Some(FileReplacement::prepare(payload, options.installed.parent().context("invalid installed path")?, &options.session.join("backup"))?);
                replacement.as_mut().unwrap().apply()?;
                status(options, "applied", "")?;
            }
            "remove" if options.uninstall => {
                // Uninstall file deletion is owned by NSIS. Once it begins,
                // an interrupted uninstaller must never relaunch a mixed tree.
                removed = true;
                status(options, "removing", "")?;
            }
            "commit" => {
                ensure!(options.uninstall || replacement.is_some(), "replacement has not been applied");
                #[cfg(windows)]
                if options.uninstall { windows::unregister_login(&options.installed)?; }
                return Ok(true);
            }
            "abort" => return Ok(false),
            _ => bail!("unsupported maintenance session command"),
        }
      }
    })();
    if matches!(completion, Ok(true)) {
        if let Some(replacement) = replacement.as_mut() { replacement.commit(); }
        drop(replacement);
        drop(guard.take());
        let committed_intent = if options.uninstall { ResumeIntent::default() } else {
            ResumeIntent {
                desktop: intent.desktop && !options.defer_app_on_commit,
                service: intent.service || login_enabled,
            }
        };
        return finish(options, "committed", &login_warning, committed_intent);
    }
    let rollback = replacement.as_mut().map(|replacement| replacement.rollback()).unwrap_or(Ok(()));
    // Drop the transaction while the lifetime fence is still held; it never
    // retries a failed explicit rollback after another process may start.
    drop(replacement);
    drop(guard.take());
    let (state, detail) = match rollback {
        Err(error) => ("repair_required", format!("application rollback failed: {error:#}")),
        Ok(()) if removed => ("repair_required", "uninstallation was interrupted after file removal began".into()),
        Ok(()) => match completion {
            Ok(false) => ("aborted", String::new()),
            Err(error) => ("error", format!("maintenance failed; application files were restored: {error:#}")),
            Ok(true) => unreachable!(),
        },
    };
    let detail = if login_warning.is_empty() { detail } else { format!("{detail} {login_warning}") };
    finish(options, state, &detail, if state == "repair_required" { ResumeIntent::default() } else { intent })
}

fn main() -> ExitCode {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(error) => { eprintln!("NomiFun maintenance: {error:#}"); return ExitCode::FAILURE; }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = status(&options, "error", &format!("maintenance failed: {error:#}"));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claiming_apply_never_consumes_a_newly_published_abort() {
        let session = tempfile::tempdir().unwrap();
        fs::write(session.path().join("command"), "apply").unwrap();
        let claimed = claim_command(session.path()).unwrap().unwrap();
        // The producer publishes abort after claim but before the consumer read.
        fs::write(session.path().join("command"), "abort").unwrap();
        assert_eq!(read_claimed_command(&claimed).unwrap(), "apply");
        assert_eq!(fs::read_to_string(session.path().join("command")).unwrap(), "abort");
        let claimed = claim_command(session.path()).unwrap().unwrap();
        assert_eq!(read_claimed_command(&claimed).unwrap(), "abort");
        assert!(claim_command(session.path()).unwrap().is_none());
    }
}
