// `ProcessError::StartLost` intentionally carries the complete last-known
// process and cleanup evidence. Keeping that evidence inline makes loss
// reporting truthful and pattern matching stable across the public API; the
// error path is cold and the extra stack space is bounded.
#![allow(clippy::result_large_err)]

mod capability;
mod command_builder;
mod io;
mod outcome;
mod platform;
mod registry;
mod request;
mod supervisor;

pub use capability::{CapabilityPolicy, SandboxPolicy};
pub use command_builder::{
    ChildProcessBuilder, ChildProcessCleanup, ManagedChildProcess, kill_process_tree,
    merge_process_path, resolve_command_in, resolve_command_path,
};
pub use io::{OutputBuffer, OutputObserver};
pub use outcome::{
    CleanupReport, EncodingMetadata, ProcessEvent, ProcessOutcome, OutputChunk, OutputCursor,
    OutputSnapshot, OutputStream, ProcessSnapshot, ProcessState, SessionId, SpawnFailure,
};
pub use request::{
    CommandSpec, ProcessError, ProcessOwner, ProcessPolicy, ProcessRequest,
    NormalizedProcessRequest, ShellKind, Transport, normalize_request,
};
pub use supervisor::{
    ProcessHandle, PollResult, ProcessSupervisor, QuiesceReport, QuiesceSessionReport,
    ShutdownReport, ShutdownSessionReport, SupervisorConfig,
};
#[cfg(windows)]
pub use platform::windows::{
    WindowsExactProcess, WindowsProcessIdentity, WindowsProcessJob, WindowsRecoveryJob,
    windows_child_process_identity, windows_process_identity,
};
#[cfg(target_os = "linux")]
pub use platform::linux_recovery::LinuxProcessGroupAnchor;
