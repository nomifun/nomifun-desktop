//! Deadlines belong to the single-attempt transport, not to an engine loop.
//! A complete frame advances the idle clock; byte drips and SSE comments do not.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use tokio::time::Sleep;

use super::{SingleAttemptFrame, SingleAttemptStream};
use crate::error::{InvokeError, InvokeErrorKind};

pub(super) fn elapsed(phase: &'static str) -> InvokeError {
    InvokeError::new(InvokeErrorKind::Timeout, phase)
}

pub(super) struct FrameDeadlineStream {
    source: Option<SingleAttemptStream>,
    idle_timeout: Duration,
    deadline: Option<Pin<Box<Sleep>>>,
}

impl FrameDeadlineStream {
    pub(super) fn new(source: SingleAttemptStream, idle_timeout: Duration) -> Self {
        Self {
            source: Some(source),
            idle_timeout,
            deadline: None,
        }
    }

    fn finish(&mut self) {
        // Drop the HTTP body and parser buffers on EOF/error, without spawning
        // a reader task, reconnecting, or manufacturing model completion.
        self.source = None;
        self.deadline = None;
    }
}

impl Stream for FrameDeadlineStream {
    type Item = Result<SingleAttemptFrame, InvokeError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.source.is_none() {
            return Poll::Ready(None);
        }
        // Start on demand: time spent by the consumer handling a previous
        // frame is backpressure, not evidence that the provider is stalled.
        if self.deadline.is_none() {
            self.deadline = Some(Box::pin(tokio::time::sleep(self.idle_timeout)));
        }
        if self
            .deadline
            .as_mut()
            .expect("deadline initialized")
            .as_mut()
            .poll(cx)
            .is_ready()
        {
            self.finish();
            return Poll::Ready(Some(Err(elapsed("provider complete-frame idle timeout"))));
        }
        let result = self
            .source
            .as_mut()
            .expect("live source")
            .as_mut()
            .poll_next(cx);
        match &result {
            Poll::Ready(Some(Ok(frame))) if frame.event != "bedrock.exception" => {
                self.deadline = None;
            }
            Poll::Ready(_) => self.finish(),
            Poll::Pending => {}
        }
        result
    }
}
