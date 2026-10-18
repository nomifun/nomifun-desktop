// `ProcessError::StartLost` intentionally carries the complete last-known
// process and cleanup evidence. Keeping that evidence inline makes loss
// reporting truthful and pattern matching stable across the public API; the
// error path is cold and the extra stack space is bounded.
#![allow(clippy::result_large_err)]

mod command_builder;
#[cfg(target_os = "macos")]
#[path = "platform/darwin_application.rs"]
mod darwin_application;
#[cfg(target_os = "macos")]
mod darwin_registered;
mod environment;
mod io;
mod outcome;
mod platform;
mod recovery;
mod registry;
mod request;
mod supervisor;

pub use environment::{environment_key_eq, is_inherited_application_environment_key, windows_powershell_module_path};

/// Whether a program selects Windows PowerShell 5.1, either through the trusted
/// system executable or the shared implicit `.ps1` launcher. Other shells and
/// programs keep their own inherited module configuration.
pub fn program_uses_windows_powershell(program: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        platform::windows::program_uses_windows_powershell(program)
    }
    #[cfg(not(windows))]
    {
        let _ = program;
        false
    }
}

pub use command_builder::{
    ChildProcessBuilder, ChildProcessCleanup, ManagedChildCleanupMetrics, ManagedChildProcess,
    kill_process_tree, managed_child_cleanup_metrics, merge_process_path, resolve_command_in,
    resolve_command_path,
};
#[cfg(target_os = "macos")]
pub use darwin_application::{
    DarwinApplicationExit, DarwinLaunchedApplicationObserver, launched_application_peer_pid,
    own_launched_application,
};
#[cfg(target_os = "macos")]
pub use darwin_registered::{
    DarwinGenerationTerminationOutcome, DarwinRegisteredChildAuthority,
    own_registered_child_process,
};
pub use io::{OutputBuffer, OutputObserver};
pub use outcome::{
    CleanupReport, EncodingMetadata, OutputChunk, OutputCursor, OutputSnapshot, OutputStream,
    ProcessOutcome, ProcessSnapshot, ProcessState, SessionId, SpawnFailure,
};
pub use platform::poller::{PlatformLifecycleMetrics, platform_lifecycle_metrics};
#[cfg(unix)]
pub use platform::startup::run_isolated_startup_probe;
pub use recovery::{
    ExactProcessIdentity, OrphanTerminationOutcome, capture_child_identity, probe_process_identity,
    same_recorded_process, terminate_verified_orphan,
};
pub use request::{
    CommandSpec, MAX_PTY_DIMENSION, NormalizedProcessRequest, ProcessError, ProcessOwner,
    ProcessPolicy, ProcessRequest, ShellKind, Transport, normalize_request,
};
pub use supervisor::{
    PollResult, ProcessHandle, ProcessSupervisor, ProcessTerminalWitness, QuiesceReport,
    QuiesceSessionReport, ShutdownReport, ShutdownSessionReport, StartupCleanupReport,
    SupervisorConfig,
};

/// Returns true only when cleanup can no longer be retried safely because its
/// exact process-tree authority was never present or has been permanently
/// lost. Timeouts, live members, and transient platform probes return false.
///
/// Callers should use this classifier instead of matching platform error text.
pub fn cleanup_authority_lost(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::Unsupported
}

#[cfg(test)]
mod cleanup_authority_tests {
    use super::cleanup_authority_lost;

    #[test]
    fn only_explicit_permanent_loss_is_classified_as_authority_loss() {
        assert!(cleanup_authority_lost(&std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "exact authority lost",
        )));
        assert!(!cleanup_authority_lost(&std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "proof remains retryable",
        )));
        assert!(!cleanup_authority_lost(&std::io::Error::other(
            "transient platform probe failed",
        )));
    }
}
#[cfg(target_os = "linux")]
pub use platform::linux_recovery::LinuxProcessGroupAnchor;
#[cfg(windows)]
pub use platform::windows::{
    WindowsConPtyCloseMetrics, WindowsExactProcess, WindowsProcessIdentity, WindowsProcessJob,
    WindowsRecoveryJob, windows_child_process_identity, windows_conpty_close_metrics,
    windows_process_identity,
};
