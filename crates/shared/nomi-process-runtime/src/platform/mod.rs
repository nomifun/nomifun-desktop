use std::{io, sync::Arc, time::Instant};

use async_trait::async_trait;

use crate::{ProcessError, NormalizedProcessRequest, OutputBuffer, Transport};

pub(crate) mod poller;

#[cfg(unix)]
pub(crate) mod unix;
#[cfg(unix)]
mod unix_pty;
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(target_os = "linux")]
pub(crate) mod linux_recovery;
#[cfg(target_os = "linux")]
mod linux_watchdog;
#[cfg(target_os = "macos")]
mod macos_watchdog;
#[cfg(target_os = "macos")]
mod macos_session;
#[cfg(unix)]
mod unix_protocol;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExitFact {
    pub(crate) code: Option<i32>,
    pub(crate) signal: Option<i32>,
    pub(crate) cleanup_errors: Vec<String>,
}

#[async_trait]
pub(crate) trait PlatformProcess: Send + Sync {
    fn pid(&self) -> u32;
    async fn write(&self, bytes: &[u8]) -> io::Result<()>;
    async fn close_stdin(&self) -> io::Result<()>;
    async fn resize(&self, _cols: u16, _rows: u16) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process transport does not support terminal resize",
        ))
    }
    async fn interrupt(&self) -> io::Result<()>;
    async fn terminate(&self) -> io::Result<()>;
    async fn force_kill(&self) -> io::Result<()>;
    async fn wait_reaped(&self, deadline: Instant) -> io::Result<ExitFact>;
}

pub(crate) struct SpawnedPlatformProcess {
    pub(crate) owner: Arc<dyn PlatformProcess>,
    /// A committed native process whose caller-facing IO setup failed.
    /// The supervisor must retain this exact owner through cleanup before
    /// returning the startup error; it is never a successful user start.
    pub(crate) startup_failure: Option<crate::SpawnFailure>,
}

#[derive(Clone)]
pub(crate) struct StartCancellation {
    #[cfg(windows)]
    native: Arc<windows::StartCancellation>,
    #[cfg(not(windows))]
    native: Arc<std::sync::atomic::AtomicBool>,
}

impl StartCancellation {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(windows)]
            native: Arc::new(windows::StartCancellation::new()),
            #[cfg(not(windows))]
            native: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel(&self) {
        #[cfg(windows)]
        self.native.cancel();
        #[cfg(not(windows))]
        self.native.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        #[cfg(windows)]
        { self.native.is_cancelled() }
        #[cfg(not(windows))]
        { self.native.load(std::sync::atomic::Ordering::Acquire) }
    }
}

pub(crate) async fn spawn(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    cancellation: StartCancellation,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    match request.transport {
        Transport::Pipe => spawn_pipe(request, output, cancellation).await,
        Transport::Pty { cols, rows } => spawn_pty(request, output, cols, rows, cancellation).await,
    }
}

pub(crate) async fn spawn_pipe(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    cancellation: StartCancellation,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    #[cfg(unix)]
    {
        unix::spawn_pipe(request, output, cancellation.native).await
    }

    #[cfg(windows)]
    {
        windows::spawn_pipe(request, output, cancellation.native).await
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (request, output, cancellation);
        Err(ProcessError::Transport {
            reason: "platform pipe adapter is pending".to_owned(),
        })
    }
}

pub(crate) async fn spawn_pty(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    cols: u16,
    rows: u16,
    cancellation: StartCancellation,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    #[cfg(unix)]
    {
        unix::spawn_pty(request, output, cols, rows, cancellation.native).await
    }

    #[cfg(windows)]
    {
        windows::spawn_pty(request, output, cols, rows, cancellation.native).await
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (request, output, cols, rows, cancellation);
        Err(ProcessError::Transport {
            reason: "platform PTY adapter is unavailable".to_owned(),
        })
    }
}
