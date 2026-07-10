use std::{io, sync::Arc, time::Instant};

use async_trait::async_trait;

use crate::{ExecutionError, NormalizedExecutionRequest, OutputBuffer, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExitFact {
    pub(crate) code: Option<i32>,
    pub(crate) signal: Option<i32>,
}

#[async_trait]
pub(crate) trait ProcessOwner: Send + Sync {
    fn pid(&self) -> u32;
    async fn write(&self, bytes: &[u8]) -> io::Result<()>;
    async fn close_stdin(&self) -> io::Result<()>;
    async fn interrupt(&self) -> io::Result<()>;
    async fn terminate(&self) -> io::Result<()>;
    async fn force_kill(&self) -> io::Result<()>;
    async fn wait_reaped(&self, deadline: Instant) -> io::Result<ExitFact>;
}

pub(crate) struct SpawnedPlatformProcess {
    pub(crate) owner: Arc<dyn ProcessOwner>,
}

pub(crate) async fn spawn(
    request: NormalizedExecutionRequest,
    output: Arc<OutputBuffer>,
) -> Result<SpawnedPlatformProcess, ExecutionError> {
    match request.transport {
        Transport::Pipe => spawn_pipe(request, output).await,
        Transport::Pty { .. } => Err(ExecutionError::Transport {
            reason: "platform PTY adapter is pending".to_owned(),
        }),
    }
}

pub(crate) async fn spawn_pipe(
    _request: NormalizedExecutionRequest,
    _output: Arc<OutputBuffer>,
) -> Result<SpawnedPlatformProcess, ExecutionError> {
    Err(ExecutionError::Transport {
        reason: "platform pipe adapter is pending".to_owned(),
    })
}

// Platform-specific process ownership adapters are introduced by later Wave A tasks.
