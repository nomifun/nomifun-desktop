//! Incremental JSON-over-SSE framing. No retry, reconnect, protocol completion
//! inference or credential ownership. Limits apply before append/JSON decoding.
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use serde_json::Value;

use super::SingleAttemptFrame;
use crate::error::InvokeError;

pub(super) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_LINES_PER_EVENT: usize = 65_536;
const POLL_WORK_BYTES: usize = 64 * 1024;
const POLL_WORK_STEPS: usize = 256;

pub(super) struct SseFrameStream<S> {
    source: Option<S>,
    pending: Vec<u8>,
    offset: usize,
    line: Vec<u8>,
    event: String,
    data: String,
    data_seen: bool,
    event_bytes: usize,
    event_lines: usize,
    max_line_bytes: usize,
    skip_lf: bool,
    first_line: bool,
    finished: bool,
}

impl<S> SseFrameStream<S> {
    pub(super) fn new(source: S, max_line_bytes: usize) -> Self {
        Self {
            source: Some(source),
            pending: Vec::new(),
            offset: 0,
            line: Vec::new(),
            event: String::new(),
            data: String::new(),
            data_seen: false,
            event_bytes: 0,
            event_lines: 0,
            max_line_bytes,
            skip_lf: false,
            first_line: true,
            finished: false,
        }
    }

    fn stop(&mut self) {
        self.finished = true;
        self.source = None;
        self.pending = Vec::new();
        self.line = Vec::new();
        self.data = String::new();
        self.event = String::new();
    }

    fn fail(
        &mut self,
        error: InvokeError,
    ) -> Poll<Option<Result<SingleAttemptFrame, InvokeError>>> {
        self.stop();
        Poll::Ready(Some(Err(error)))
    }

    fn consume_line(&mut self) -> Result<Option<SingleAttemptFrame>, InvokeError> {
        self.event_lines += 1;
        self.event_bytes = self
            .event_bytes
            .saturating_add(self.line.len())
            .saturating_add(1);
        if self.event_bytes > MAX_FRAME_BYTES || self.event_lines > MAX_LINES_PER_EVENT {
            return Err(InvokeError::parse(
                "provider SSE event exceeds the framing limit",
            ));
        }
        let line = std::mem::take(&mut self.line);
        // SSE permits one UTF-8 BOM at the start, including across HTTP chunks.
        let bytes = if self.first_line {
            line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&line)
        } else {
            &line
        };
        self.first_line = false;
        let line = std::str::from_utf8(bytes)
            .map_err(|_| InvokeError::parse("provider SSE line is not UTF-8"))?;
        if line.is_empty() {
            return self.finish_event();
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = value.to_owned(),
            "data" => {
                let separator = usize::from(self.data_seen);
                if self
                    .data
                    .len()
                    .saturating_add(separator)
                    .saturating_add(value.len())
                    > MAX_FRAME_BYTES
                {
                    return Err(InvokeError::parse(
                        "provider SSE data exceeds the framing limit",
                    ));
                }
                if self.data_seen {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.data_seen = true;
            }
            // id/retry/extension fields never authorize reconnect or replay.
            _ => {}
        }
        Ok(None)
    }

    fn finish_event(&mut self) -> Result<Option<SingleAttemptFrame>, InvokeError> {
        self.event_bytes = 0;
        self.event_lines = 0;
        let event = std::mem::take(&mut self.event);
        if !std::mem::take(&mut self.data_seen) {
            // Comment-only and event-only records are not JSON model events.
            return Ok(None);
        }
        let data = std::mem::take(&mut self.data);
        if data.trim() == "[DONE]" {
            if !matches!(event.as_str(), "" | "message" | "done") {
                return Err(InvokeError::parse(
                    "provider SSE terminal marker contradicts its event name",
                ));
            }
            self.stop();
            return Ok(Some(SingleAttemptFrame {
                event: "done".to_owned(),
                data: Value::Object(Default::default()),
            }));
        }
        let data = serde_json::from_str(&data)
            .map_err(|_| InvokeError::parse("provider SSE data is not valid JSON"))?;
        Ok(Some(SingleAttemptFrame {
            event: if event.is_empty() {
                "message".to_owned()
            } else {
                event
            },
            data,
        }))
    }
}

impl<S> Stream for SseFrameStream<S>
where
    S: Stream<Item = Result<Vec<u8>, InvokeError>> + Unpin,
{
    type Item = Result<SingleAttemptFrame, InvokeError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let mut work_bytes = 0usize;
        let mut work_steps = 0usize;
        loop {
            if this.finished {
                return Poll::Ready(None);
            }
            // Ready-only sources and comment floods must yield so cancellation
            // and other Sessions can run. This does not reset any idle timeout.
            if work_bytes >= POLL_WORK_BYTES || work_steps >= POLL_WORK_STEPS {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            work_steps += 1;
            if this.offset == this.pending.len() {
                this.pending = Vec::new();
                this.offset = 0;
                match Pin::new(this.source.as_mut().expect("unfinished source")).poll_next(cx) {
                    Poll::Ready(Some(Ok(bytes))) => {
                        if bytes.len() > MAX_FRAME_BYTES {
                            return this.fail(InvokeError::parse(
                                "provider SSE transport chunk exceeds the framing limit",
                            ));
                        }
                        this.pending = bytes;
                        continue;
                    }
                    Poll::Ready(Some(Err(error))) => return this.fail(error),
                    Poll::Ready(None) => {
                        // A blank line commits an SSE event. EOF must not turn
                        // an unfinished JSON record into a successful terminal.
                        let incomplete =
                            !this.line.is_empty() || this.data_seen || !this.event.is_empty();
                        this.stop();
                        return if incomplete {
                            Poll::Ready(Some(Err(InvokeError::parse(
                                "provider SSE stream ended before the event delimiter",
                            ))))
                        } else {
                            Poll::Ready(None)
                        };
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }
            // CRLF is one terminator even when split across transport chunks;
            // lone CR and LF are also valid SSE line endings.
            if this.skip_lf {
                this.skip_lf = false;
                if this.pending[this.offset] == b'\n' {
                    this.offset += 1;
                    work_bytes += 1;
                    continue;
                }
            }
            let remaining = &this.pending[this.offset..];
            let delimiter = remaining
                .iter()
                .position(|byte| matches!(byte, b'\r' | b'\n'));
            let count = delimiter.unwrap_or(remaining.len());
            if this.line.len().saturating_add(count) > this.max_line_bytes {
                return this.fail(InvokeError::parse(
                    "provider SSE line exceeds the configured limit",
                ));
            }
            this.line.extend_from_slice(&remaining[..count]);
            this.offset += count;
            work_bytes = work_bytes.saturating_add(count);
            if delimiter.is_some() {
                this.skip_lf = this.pending[this.offset] == b'\r';
                this.offset += 1;
                work_bytes += 1;
                match this.consume_line() {
                    Ok(Some(frame)) => return Poll::Ready(Some(Ok(frame))),
                    Ok(None) => {}
                    Err(error) => return this.fail(error),
                }
            }
        }
    }
}
