//! Renewable loopback capability client shared by all scoped stdio bridges.
//!
//! A bridge receives one JSON bootstrap containing short-lived access plus a
//! process-scoped renewal proof. It never receives the backend root issuer.
//! Every child start renews immediately (so an ACP/Nomi respawn can reuse an
//! old env safely), subsequent refreshes are single-flight, and a 401 retries
//! exactly once after forced renewal.

use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{self, Write};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use nomifun_api_types::{
    MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY, ScopedMcpChildBootstrap,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_RENEWAL_MARGIN_SECS,
    LOOPBACK_CAPABILITY_REVOKE_PATH, LoopbackCapabilityAccess,
    LoopbackCapabilityClaims, LoopbackCapabilityError,
    LoopbackCapabilityRenewalRequest, unix_time_secs,
};
use rmcp::model::{
    CallToolResult, ClientJsonRpcMessage, ClientNotification, ClientRequest, Content, ErrorCode,
    ErrorData, GetExtensions, JsonRpcMessage, NumberOrString, RequestId, ServerInfo,
    ServerJsonRpcMessage, ServerResult,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer, Service};
use rmcp::transport::Transport;
use rmcp::transport::async_rw::JsonRpcMessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, FramedRead, FramedWrite};
use tokio_util::sync::CancellationToken;

type ScopedAccess<S> = LoopbackCapabilityAccess<LoopbackCapabilityClaims<S>>;

const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";

/// A Platform Gateway child carries broad structured platform calls, so it may
/// retain a little more work than the browser-only bridge. This is a per-child
/// admission bound, never a process-global concurrency cap.
pub(crate) const MAX_GATEWAY_STDIO_ACTIVE_REQUESTS: usize = 8;

/// A browser-only ACP child is deliberately serial. Concurrency for one
/// user-visible task comes from its independently fenced sibling children and
/// from the Browser Hub, rather than from retaining multiple large JSON values
/// in every proxy process.
pub(crate) const MAX_BROWSER_STDIO_ACTIVE_REQUESTS: usize = 1;

/// Notifications do not receive JSON-RPC responses, so they use a separate,
/// small control budget. This keeps cancellation available while eight browser
/// requests are active without giving notification floods an unbounded spawn
/// path through rmcp's per-message tasks.
const MAX_STDIO_ACTIVE_NOTIFICATIONS: usize = 8;

/// Browser requests are normally far smaller (crawl URLs are independently
/// bounded by the Hub), but the shared Platform Gateway also carries rich
/// structured tool arguments. The fixed wire limit is a structural safety fuse,
/// not a process-wide or machine-wide memory target.
pub(crate) const MAX_GATEWAY_STDIO_INPUT_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Browser tool arguments are bounded again by the Hub (URL count, schema
/// depth, strings, etc.). 256 KiB leaves ample protocol headroom without
/// allowing every child in a task family to retain a multi-megabyte `Value`.
pub(crate) const MAX_BROWSER_STDIO_INPUT_FRAME_BYTES: usize = 256 * 1024;

/// Tool results can contain screenshots or a bounded crawl batch. Encoding is
/// performed through a capped writer so an oversized result cannot first create
/// an unbounded temporary JSON buffer.
pub(crate) const MAX_GATEWAY_STDIO_OUTPUT_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Browser crawl/screenshot results are capped before this encoder. 20 MiB
/// preserves headroom around the 16 MiB loopback body fuse while bounding a
/// blocked browser-family wire envelope independently of Gateway traffic.
pub(crate) const MAX_BROWSER_STDIO_OUTPUT_FRAME_BYTES: usize = 20 * 1024 * 1024;

/// Control notifications should contain only request ids/progress metadata.
/// Keeping them small preserves cancellation without letting the reserved
/// notification slots become another large-Value retention path.
const MAX_STDIO_NOTIFICATION_FRAME_BYTES: usize = 64 * 1024;
const MAX_STDIO_REQUEST_ID_BYTES: usize = 4 * 1024;
const STDIO_CAPACITY_ERROR_CODE: ErrorCode = ErrorCode(-32001);
const STDIO_SESSION_LIFETIME_ERROR_CODE: ErrorCode = ErrorCode(-32004);

/// rmcp 1.7 retains completed response-send task entries until the transport
/// closes. Cycling a short-lived bridge after this many admitted requests puts
/// a hard ceiling on that third-party bookkeeping even for a perfectly behaved
/// client. Exhaustion is intentionally fail-closed: an MCP supervisor may
/// respawn a fresh capability-bound child, but this process does not claim that
/// every external client supports transparent mid-session restart.
const MAX_STDIO_ADMITTED_REQUESTS_PER_CHILD: usize = 1_024;

/// The deadline covers waiting for the writer mutex *and* encoding/flushing the
/// frame. A client that stops reading stdout can therefore never pin a child.
const STDIO_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Covers the entire service future. Request cancellation is selected first;
/// dropping the future also drops any in-flight reqwest body stream promptly.
const STDIO_HANDLER_TIMEOUT: Duration = Duration::from_secs(90);

/// The HTTP stream is accumulated only up to this decompressed-byte ceiling.
/// Browser results are independently encoded through the 20 MiB stdio cap.
const MAX_LOOPBACK_TOOL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LOOPBACK_CONTROL_RESPONSE_BYTES: usize = 1024 * 1024;

/// Maximum browser-proxy payload simultaneously retained on blocked stdio
/// wires for one `(user, conversation)` task family. This deliberately does
/// not estimate allocator/serde/runtime overhead; those are separately bounded
/// by the child count, serial admission, fixed runtime, and session fuse.
pub(crate) const MAX_BROWSER_TASK_FAMILY_STDIO_WIRE_BYTES: usize =
    MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY
        * (MAX_BROWSER_STDIO_ACTIVE_REQUESTS
            * (MAX_BROWSER_STDIO_INPUT_FRAME_BYTES
                + 1
                + MAX_BROWSER_STDIO_OUTPUT_FRAME_BYTES)
            + MAX_STDIO_ACTIVE_NOTIFICATIONS * (MAX_STDIO_NOTIFICATION_FRAME_BYTES + 1));

#[derive(Clone)]
pub(crate) struct ProcessRequestBudget {
    requests: Arc<AtomicUsize>,
    notifications: Arc<AtomicUsize>,
    max_requests: usize,
    max_notifications: usize,
    shutdown: CancellationToken,
}

impl Default for ProcessRequestBudget {
    fn default() -> Self {
        Self::new(
            MAX_GATEWAY_STDIO_ACTIVE_REQUESTS,
            MAX_STDIO_ACTIVE_NOTIFICATIONS,
        )
    }
}

impl ProcessRequestBudget {
    pub(crate) fn browser() -> Self {
        Self::new(
            MAX_BROWSER_STDIO_ACTIVE_REQUESTS,
            MAX_STDIO_ACTIVE_NOTIFICATIONS,
        )
    }

    fn new(max_requests: usize, max_notifications: usize) -> Self {
        assert!(max_requests > 0);
        assert!(max_notifications > 0);
        Self {
            requests: Arc::new(AtomicUsize::new(0)),
            notifications: Arc::new(AtomicUsize::new(0)),
            max_requests,
            max_notifications,
            shutdown: CancellationToken::new(),
        }
    }

    fn try_acquire_request(&self) -> Option<ProcessRequestPermit> {
        try_acquire_counter(&self.requests, self.max_requests)
    }

    fn try_acquire_notification(&self) -> Option<ProcessRequestPermit> {
        try_acquire_counter(&self.notifications, self.max_notifications)
    }

    fn max_requests(&self) -> usize {
        self.max_requests
    }

    fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    #[cfg(test)]
    fn active_requests(&self) -> usize {
        self.requests.load(Ordering::Acquire)
    }
}

fn try_acquire_counter(
    counter: &Arc<AtomicUsize>,
    limit: usize,
) -> Option<ProcessRequestPermit> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= limit {
            return None;
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                return Some(ProcessRequestPermit(Arc::new(RequestPermitInner {
                    counter: counter.clone(),
                })));
            }
            Err(actual) => current = actual,
        }
    }
}

/// Clone is intentional: rmcp clones the initialize request/context. One
/// reservation is refunded only when the final clone is dropped.
#[derive(Clone)]
pub(crate) struct ProcessRequestPermit(Arc<RequestPermitInner>);

impl ProcessRequestPermit {
    fn same_reservation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct RequestPermitInner {
    counter: Arc<AtomicUsize>,
}

impl Drop for RequestPermitInner {
    fn drop(&mut self) {
        let previous = self.counter.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "stdio request admission underflow");
    }
}

/// Proof held by a concrete handler. For transport-dispatched requests the
/// panic/cancellation shield owns the real reservation and this is only a
/// marker. Direct Service invocations acquire a real fallback reservation.
pub(crate) struct StdioHandlerRequestPermit {
    _fallback: Option<ProcessRequestPermit>,
}

#[derive(Clone, Copy)]
struct StdioTransportAdmissionMarker;

pub(crate) fn take_stdio_request_permit(
    context: &mut RequestContext<RoleServer>,
    budget: &ProcessRequestBudget,
) -> Result<StdioHandlerRequestPermit, ErrorData> {
    if context
        .extensions
        .remove::<StdioTransportAdmissionMarker>()
        .is_some()
    {
        return Ok(StdioHandlerRequestPermit { _fallback: None });
    }
    budget
        .try_acquire_request()
        .map(|permit| StdioHandlerRequestPermit {
            _fallback: Some(permit),
        })
        .ok_or_else(|| stdio_capacity_error(budget.max_requests()))
}

fn stdio_capacity_error(max_active_requests: usize) -> ErrorData {
    ErrorData::new(
        STDIO_CAPACITY_ERROR_CODE,
        "stdio task request capacity is full; retry after an active request completes",
        Some(serde_json::json!({
            "code": "stdio_request_capacity",
            "retryable": true,
            "max_active_requests": max_active_requests,
        })),
    )
}

fn stdio_session_lifetime_error(max_requests: usize) -> ErrorData {
    ErrorData::new(
        STDIO_SESSION_LIFETIME_ERROR_CODE,
        "stdio child request lifetime is exhausted; restart this capability-bound child",
        Some(serde_json::json!({
            "code": "stdio_session_lifetime_exhausted",
            "retryable": true,
            "max_admitted_requests": max_requests,
        })),
    )
}

#[derive(Clone)]
struct ActiveRequestRegistry {
    budget: ProcessRequestBudget,
    requests: Arc<StdMutex<HashMap<RequestId, ProcessRequestPermit>>>,
}

#[derive(Debug)]
enum RequestAdmissionError {
    Capacity,
    Duplicate,
}

impl ActiveRequestRegistry {
    fn new(budget: ProcessRequestBudget) -> Self {
        Self {
            budget,
            requests: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    fn try_admit(
        &self,
        request_id: &RequestId,
    ) -> Result<TransportRequestGuard, RequestAdmissionError> {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if requests.contains_key(request_id) {
            return Err(RequestAdmissionError::Duplicate);
        }
        let permit = self
            .budget
            .try_acquire_request()
            .ok_or(RequestAdmissionError::Capacity)?;
        requests.insert(request_id.clone(), permit.clone());
        drop(requests);
        Ok(TransportRequestGuard(Arc::new(
            TransportRequestGuardInner {
                registry: self.clone(),
                request_id: request_id.clone(),
                permit,
                handed_to_response: AtomicBool::new(false),
            },
        )))
    }

    fn begin_completion(&self, request_id: &RequestId) -> Option<RequestCompletionGuard> {
        let permit = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(request_id)
            .cloned()?;
        Some(RequestCompletionGuard {
            registry: self.clone(),
            request_id: request_id.clone(),
            permit,
        })
    }

    fn remove_if_same(&self, request_id: &RequestId, expected: &ProcessRequestPermit) {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if requests
            .get(request_id)
            .is_some_and(|actual| actual.same_reservation(expected))
        {
            requests.remove(request_id);
        }
    }

    #[cfg(test)]
    fn contains(&self, request_id: &RequestId) -> bool {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(request_id)
    }
}

/// Lives in the rmcp request extensions until the shield takes ownership. If
/// the handler task is dropped before it can produce a response, the final
/// clone removes exactly its own registry entry and refunds the active slot.
#[derive(Clone)]
struct TransportRequestGuard(Arc<TransportRequestGuardInner>);

impl TransportRequestGuard {
    fn handoff_to_response(&self) {
        self.0.handed_to_response.store(true, Ordering::Release);
    }
}

struct TransportRequestGuardInner {
    registry: ActiveRequestRegistry,
    request_id: RequestId,
    permit: ProcessRequestPermit,
    handed_to_response: AtomicBool,
}

impl Drop for TransportRequestGuardInner {
    fn drop(&mut self) {
        if !self.handed_to_response.load(Ordering::Acquire) {
            self.registry
                .remove_if_same(&self.request_id, &self.permit);
        }
    }
}

struct RequestCompletionGuard {
    registry: ActiveRequestRegistry,
    request_id: RequestId,
    permit: ProcessRequestPermit,
}

impl Drop for RequestCompletionGuard {
    fn drop(&mut self) {
        self.registry
            .remove_if_same(&self.request_id, &self.permit);
    }
}

struct InboundFrame<T> {
    message: T,
    encoded_len: usize,
}

struct BoundedInboundCodec<T> {
    inner: JsonRpcMessageCodec<T>,
}

impl<T> BoundedInboundCodec<T> {
    fn new(max_length: usize) -> Self {
        Self {
            inner: JsonRpcMessageCodec::new_with_max_length(max_length),
        }
    }

    fn wrap_decoded(
        before: usize,
        after: usize,
        decoded: Option<T>,
    ) -> Option<InboundFrame<T>> {
        decoded.map(|message| InboundFrame {
            message,
            encoded_len: before.saturating_sub(after),
        })
    }
}

impl<T: DeserializeOwned> Decoder for BoundedInboundCodec<T> {
    type Item = InboundFrame<T>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let before = src.len();
        let decoded = self.inner.decode(src).map_err(io::Error::from)?;
        Ok(Self::wrap_decoded(before, src.len(), decoded))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let before = src.len();
        let decoded = self.inner.decode_eof(src).map_err(io::Error::from)?;
        Ok(Self::wrap_decoded(before, src.len(), decoded))
    }
}

struct BoundedOutboundCodec<T> {
    max_length: usize,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> BoundedOutboundCodec<T> {
    fn new(max_length: usize) -> Self {
        Self {
            max_length,
            marker: std::marker::PhantomData,
        }
    }
}

struct CappedBytesWriter<'a> {
    output: &'a mut BytesMut,
    start: usize,
    max_length: usize,
}

impl Write for CappedBytesWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.output.len().saturating_sub(self.start);
        if bytes.len() > self.max_length.saturating_sub(written) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "maximum stdio output frame length exceeded",
            ));
        }
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<T: Serialize> Encoder<T> for BoundedOutboundCodec<T> {
    type Error = io::Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let start = dst.len();
        let result = {
            let mut writer = CappedBytesWriter {
                output: dst,
                start,
                // Reserve the final byte for the newline frame delimiter so
                // the configured output limit covers the entire wire frame.
                max_length: self.max_length.saturating_sub(1),
            };
            serde_json::to_writer(&mut writer, &item)
        };
        if let Err(error) = result {
            dst.truncate(start);
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        }
        dst.extend_from_slice(b"\n");
        Ok(())
    }
}

struct TransportFailure {
    cancellation: CancellationToken,
}

impl TransportFailure {
    fn new(cancellation: CancellationToken) -> Self {
        Self { cancellation }
    }

    fn fail(&self) {
        self.cancellation.cancel();
    }

    fn is_failed(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
}

type StdioWriter<W> = FramedWrite<W, BoundedOutboundCodec<ServerJsonRpcMessage>>;

async fn send_stdio_message<W>(
    writer: Arc<Mutex<Option<StdioWriter<W>>>>,
    failure: Arc<TransportFailure>,
    write_timeout: Duration,
    message: ServerJsonRpcMessage,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if failure.is_failed() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "stdio transport failed closed",
        ));
    }

    let result = tokio::time::timeout(write_timeout, async {
        // The timeout starts before acquiring the mutex: queued writers cannot
        // extend their deadline by waiting behind a blocked flush.
        let mut slot = writer.lock().await;
        if failure.is_failed() {
            slot.take();
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "stdio transport failed closed",
            ));
        }
        let result = match slot.as_mut() {
            Some(writer) => writer.send(message).await,
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "stdio transport is closed",
            )),
        };
        if result.is_err() {
            // Discard any codec/socket buffer after an encode or flush error.
            slot.take();
        }
        result
    })
    .await;

    match result {
        Ok(result) => {
            if result.is_err() {
                failure.fail();
            }
            result
        }
        Err(_) => {
            failure.fail();
            // Timeout cancels the lock/send future and releases its guard. A
            // racing waiter rechecks `failure` and performs the same discard.
            if let Ok(mut slot) = writer.try_lock() {
                slot.take();
            }
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "stdio output write deadline exceeded",
            ))
        }
    }
}

pub(crate) struct BoundedStdioTransport<R, W> {
    reader: FramedRead<R, BoundedInboundCodec<ClientJsonRpcMessage>>,
    writer: Arc<Mutex<Option<StdioWriter<W>>>>,
    admissions: ActiveRequestRegistry,
    budget: ProcessRequestBudget,
    output_active: Arc<AtomicUsize>,
    output_limit: usize,
    failure: Arc<TransportFailure>,
    notification_frame_limit: usize,
    admitted_requests: usize,
    request_lifetime_limit: usize,
    write_timeout: Duration,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead,
    W: AsyncWrite + Unpin,
{
    fn new_with_limits(
        read: R,
        write: W,
        budget: ProcessRequestBudget,
        input_frame_limit: usize,
        output_frame_limit: usize,
    ) -> Self {
        Self::new_with_all_limits(
            read,
            write,
            budget,
            input_frame_limit,
            output_frame_limit,
            MAX_STDIO_ADMITTED_REQUESTS_PER_CHILD,
            STDIO_WRITE_TIMEOUT,
        )
    }

    fn new_with_all_limits(
        read: R,
        write: W,
        budget: ProcessRequestBudget,
        input_frame_limit: usize,
        output_frame_limit: usize,
        request_lifetime_limit: usize,
        write_timeout: Duration,
    ) -> Self {
        assert!(input_frame_limit > 0);
        assert!(output_frame_limit > 0);
        assert!(request_lifetime_limit > 0);
        assert!(!write_timeout.is_zero());
        let output_limit = budget.max_requests();
        let failure = Arc::new(TransportFailure::new(budget.shutdown_token()));
        Self {
            reader: FramedRead::new(read, BoundedInboundCodec::new(input_frame_limit)),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                write,
                BoundedOutboundCodec::new(output_frame_limit),
            )))),
            admissions: ActiveRequestRegistry::new(budget.clone()),
            budget,
            output_active: Arc::new(AtomicUsize::new(0)),
            output_limit,
            failure,
            notification_frame_limit: MAX_STDIO_NOTIFICATION_FRAME_BYTES.min(input_frame_limit),
            admitted_requests: 0,
            request_lifetime_limit,
            write_timeout,
        }
    }

    async fn write_direct(&self, message: ServerJsonRpcMessage) -> io::Result<()> {
        send_stdio_message(
            self.writer.clone(),
            self.failure.clone(),
            self.write_timeout,
            message,
        )
        .await
    }

    async fn reject_request(
        &self,
        request_id: Option<RequestId>,
        error: ErrorData,
    ) -> io::Result<()> {
        self.write_direct(ServerJsonRpcMessage::error(error, request_id))
            .await
    }
}

pub(crate) fn bounded_stdio_transport(
    budget: ProcessRequestBudget,
) -> BoundedStdioTransport<tokio::io::Stdin, tokio::io::Stdout> {
    BoundedStdioTransport::new_with_limits(
        tokio::io::stdin(),
        tokio::io::stdout(),
        budget,
        MAX_GATEWAY_STDIO_INPUT_FRAME_BYTES,
        MAX_GATEWAY_STDIO_OUTPUT_FRAME_BYTES,
    )
}

pub(crate) fn bounded_browser_stdio_transport(
    budget: ProcessRequestBudget,
) -> BoundedStdioTransport<tokio::io::Stdin, tokio::io::Stdout> {
    BoundedStdioTransport::new_with_limits(
        tokio::io::stdin(),
        tokio::io::stdout(),
        budget,
        MAX_BROWSER_STDIO_INPUT_FRAME_BYTES,
        MAX_BROWSER_STDIO_OUTPUT_FRAME_BYTES,
    )
}

fn request_id_is_bounded(request_id: &RequestId) -> bool {
    match request_id {
        NumberOrString::Number(_) => true,
        NumberOrString::String(value) => value.len() <= MAX_STDIO_REQUEST_ID_BYTES,
    }
}

impl<R, W> Transport<RoleServer> for BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Sync + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = self.writer.clone();
        let failure = self.failure.clone();
        let write_timeout = self.write_timeout;
        let completed = match &item {
            JsonRpcMessage::Response(response) => self.admissions.begin_completion(&response.id),
            JsonRpcMessage::Error(error) => error
                .id
                .as_ref()
                .and_then(|request_id| self.admissions.begin_completion(request_id)),
            _ => None,
        };
        let output_permit = try_acquire_counter(&self.output_active, self.output_limit);
        async move {
            let _completed = completed;
            let _output_permit = match output_permit {
                Some(permit) => permit,
                None => {
                    failure.fail();
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "stdio output capacity is full",
                    ));
                }
            };
            send_stdio_message(writer, failure, write_timeout, item).await
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            if self.failure.is_failed() {
                return None;
            }
            let frame = tokio::select! {
                biased;
                _ = self.failure.cancelled() => return None,
                frame = self.reader.next() => frame,
            };
            let InboundFrame {
                mut message,
                encoded_len,
            } = match frame {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => {
                    eprintln!("[scoped-stdio] input rejected; closing session: {error}");
                    self.failure.fail();
                    return None;
                }
                None => {
                    // rmcp does not cancel detached handlers merely because
                    // receive returned EOF. Propagate child shutdown through
                    // the same level-triggered token the shield selects.
                    self.failure.fail();
                    return None;
                }
            };

            match &message {
                JsonRpcMessage::Request(request) if !request_id_is_bounded(&request.id) => {
                    drop(message);
                    let error = ErrorData::invalid_request(
                        "stdio JSON-RPC request id exceeds the fixed byte limit",
                        None,
                    );
                    if self.reject_request(None, error).await.is_err() {
                        return None;
                    }
                    continue;
                }
                JsonRpcMessage::Request(request) => {
                    let request_id = request.id.clone();
                    if self.admitted_requests >= self.request_lifetime_limit {
                        drop(message);
                        let result = self
                            .reject_request(
                                Some(request_id),
                                stdio_session_lifetime_error(self.request_lifetime_limit),
                            )
                            .await;
                        // Even a successfully written fuse response closes the
                        // child so rmcp's completed JoinSet cannot grow again.
                        self.failure.fail();
                        if result.is_err() {
                            return None;
                        }
                        return None;
                    }
                    match self.admissions.try_admit(&request_id) {
                        Ok(guard) => {
                            self.admitted_requests += 1;
                            message.insert_extension(guard);
                        }
                        Err(RequestAdmissionError::Capacity) => {
                            drop(message);
                            if self
                                .reject_request(
                                    Some(request_id),
                                    stdio_capacity_error(self.budget.max_requests()),
                                )
                                .await
                                .is_err()
                            {
                                return None;
                            }
                            continue;
                        }
                        Err(RequestAdmissionError::Duplicate) => {
                            drop(message);
                            if self
                                .reject_request(
                                    Some(request_id),
                                    ErrorData::invalid_request(
                                        "duplicate in-flight stdio JSON-RPC request id",
                                        None,
                                    ),
                                )
                                .await
                                .is_err()
                            {
                                return None;
                            }
                            continue;
                        }
                    }
                }
                JsonRpcMessage::Notification(_) => {
                    if encoded_len > self.notification_frame_limit {
                        eprintln!(
                            "[scoped-stdio] oversized control notification dropped: {encoded_len} bytes"
                        );
                        continue;
                    }
                    let Some(permit) = self.budget.try_acquire_notification() else {
                        eprintln!("[scoped-stdio] control notification capacity is full; dropped");
                        continue;
                    };
                    message.insert_extension(permit);
                }
                JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_) => {}
            }
            return Some(message);
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.failure.fail();
        let writer = self.writer.clone();
        match tokio::time::timeout(self.write_timeout, async move {
            let mut slot = writer.lock().await;
            let Some(mut framed) = slot.take() else {
                return Ok(());
            };
            drop(slot);
            framed.close().await
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                if let Ok(mut slot) = self.writer.try_lock() {
                    slot.take();
                }
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdio close deadline exceeded",
                ))
            }
        }
    }
}

pub(crate) struct PanicShieldService<S> {
    inner: S,
    handler_timeout: Duration,
    child_shutdown: CancellationToken,
}

pub(crate) fn panic_shield_stdio_service<S>(
    inner: S,
    budget: ProcessRequestBudget,
) -> PanicShieldService<S> {
    PanicShieldService {
        inner,
        handler_timeout: STDIO_HANDLER_TIMEOUT,
        child_shutdown: budget.shutdown_token(),
    }
}

#[cfg(test)]
fn panic_shield_stdio_service_with_timeout<S>(
    inner: S,
    budget: ProcessRequestBudget,
    handler_timeout: Duration,
) -> PanicShieldService<S> {
    PanicShieldService {
        inner,
        handler_timeout,
        child_shutdown: budget.shutdown_token(),
    }
}

fn stdio_handler_cancelled_error() -> ErrorData {
    ErrorData::internal_error(
        "stdio request was cancelled; in-flight I/O was dropped",
        Some(serde_json::json!({
            "code": "stdio_request_cancelled",
            "retryable": true,
            "side_effects_reverted": false,
        })),
    )
}

fn stdio_handler_deadline_error(timeout: Duration) -> ErrorData {
    ErrorData::internal_error(
        "stdio request deadline exceeded; in-flight I/O was dropped",
        Some(serde_json::json!({
            "code": "stdio_request_deadline",
            "retryable": true,
            "deadline_ms": timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            "side_effects_reverted": false,
        })),
    )
}

impl<S> Service<RoleServer> for PanicShieldService<S>
where
    S: Service<RoleServer>,
{
    fn handle_request(
        &self,
        mut request: ClientRequest,
        mut context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ServerResult, ErrorData>> + Send + '_ {
        let handler_timeout = self.handler_timeout;
        let child_shutdown = self.child_shutdown.clone();
        async move {
            // Regular rmcp requests move extensions into the context. During
            // initialize rmcp clones both request and context, so remove and
            // retain both copies until one terminal outcome is chosen.
            let mut admissions = Vec::with_capacity(2);
            if let Some(guard) = context.extensions.remove::<TransportRequestGuard>() {
                admissions.push(guard);
            }
            if let Some(guard) = request
                .extensions_mut()
                .remove::<TransportRequestGuard>()
            {
                admissions.push(guard);
            }
            if !admissions.is_empty() {
                context
                    .extensions
                    .insert(StdioTransportAdmissionMarker);
            }

            let cancellation = context.ct.clone();
            let handler = AssertUnwindSafe(self.inner.handle_request(request, context))
                .catch_unwind();
            tokio::pin!(handler);
            let deadline = tokio::time::sleep(handler_timeout);
            tokio::pin!(deadline);
            let (result, handoff_to_response) = tokio::select! {
                biased;
                // There is no usable transport left, so do not hand the
                // reservation to a response that cannot be written.
                _ = child_shutdown.cancelled() => (Err(stdio_handler_cancelled_error()), false),
                _ = cancellation.cancelled() => (Err(stdio_handler_cancelled_error()), true),
                _ = &mut deadline => (Err(stdio_handler_deadline_error(handler_timeout)), true),
                outcome = &mut handler => (match outcome {
                    Ok(result) => result,
                    Err(_) => Err(ErrorData::internal_error(
                        "stdio request handler panicked; request authority was released",
                        Some(serde_json::json!({"code": "stdio_request_panic"})),
                    )),
                }, true),
            };

            // Every selected branch returns a bounded protocol response. Keep
            // the exact request-id fence until that response write completes.
            if handoff_to_response {
                for admission in &admissions {
                    admission.handoff_to_response();
                }
            }
            drop(admissions);
            result
        }
    }

    fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + Send + '_ {
        let handler_timeout = self.handler_timeout;
        let child_shutdown = self.child_shutdown.clone();
        async move {
            let handler = AssertUnwindSafe(self.inner.handle_notification(notification, context))
                .catch_unwind();
            tokio::pin!(handler);
            let deadline = tokio::time::sleep(handler_timeout);
            tokio::pin!(deadline);
            tokio::select! {
                biased;
                _ = child_shutdown.cancelled() => Err(stdio_handler_cancelled_error()),
                _ = &mut deadline => Err(ErrorData::internal_error(
                    "stdio notification handler deadline exceeded",
                    Some(serde_json::json!({"code": "stdio_notification_deadline"})),
                )),
                outcome = &mut handler => match outcome {
                    Ok(result) => result,
                    Err(_) => Err(ErrorData::internal_error(
                    "stdio notification handler panicked; notification authority was released",
                    Some(serde_json::json!({"code": "stdio_notification_panic"})),
                    )),
                },
            }
        }
    }

    fn get_info(&self) -> ServerInfo {
        self.inner.get_info()
    }
}

fn valid_idempotency_key(value: &str) -> bool {
    nomifun_common::is_visible_ascii_key(value, nomifun_common::MAX_IDEMPOTENCY_KEY_LEN)
}

/// How a forwarded tool POST may be retried at the transport level.
#[derive(Clone, Copy)]
enum ToolDeliveryPolicy<'a> {
    /// Retry any transport failure. Safe only when the endpoint's tools are
    /// idempotent, or when `idempotency_key` is set and the server dedupes on
    /// it (the Platform Gateway does).
    AtLeastOnce { idempotency_key: Option<&'a str> },
    /// Never re-POST once the request may have been delivered. Only
    /// connection-setup failures — where the request provably never left this
    /// process — are retried. Used for endpoints that execute non-idempotent
    /// side effects and have no idempotency-key dedup (the Browser Hub
    /// bridge: click/type/press_key can be an irreversible submit).
    AtMostOnce,
}

impl ToolDeliveryPolicy<'_> {
    fn idempotency_key(&self) -> Option<&str> {
        match self {
            Self::AtLeastOnce { idempotency_key } => *idempotency_key,
            Self::AtMostOnce => None,
        }
    }
}

/// Structured outcome of forwarding a tool call over the loopback bridge.
///
/// The distinction is derived from the HTTP status and the gateway's JSON
/// response envelope (a top-level `error` member), never from words appearing
/// in the rendered tool text.  MCP bridges use this to set protocol-level
/// `CallToolResult.isError` through [`into_mcp_tool_result`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardToolOutcome {
    Success(String),
    Error(String),
}

impl ForwardToolOutcome {
    pub(crate) fn into_parts(self) -> (String, bool) {
        match self {
            Self::Success(text) => (text, false),
            Self::Error(text) => (text, true),
        }
    }
}

/// Convert a structured loopback outcome into the MCP wire result while
/// preserving its error bit. A capability may optionally attach
/// `_mcp_images: [{"mime_type","data"}]`; those entries become MCP image
/// content and are removed from the text payload.
pub(crate) fn into_mcp_tool_result(outcome: ForwardToolOutcome) -> CallToolResult {
    let (text, is_error) = outcome.into_parts();
    if !text.contains("_mcp_images") {
        return call_tool_result(vec![Content::text(text)], is_error);
    }

    let parsed: Option<serde_json::Value> = serde_json::from_str(&text).ok();
    let images: Vec<Content> = parsed
        .as_ref()
        .and_then(|value| value.get("_mcp_images"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|image| {
                    let data = image.get("data").and_then(serde_json::Value::as_str)?;
                    let mime = image
                        .get("mime_type")
                        .and_then(serde_json::Value::as_str)?;
                    Some(Content::image(data.to_owned(), mime.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    if images.is_empty() {
        return call_tool_result(vec![Content::text(text)], is_error);
    }

    let text_out = match parsed {
        Some(serde_json::Value::Object(mut map)) => {
            map.remove("_mcp_images");
            serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or(text)
        }
        _ => text,
    };
    let mut content = vec![Content::text(text_out)];
    content.extend(images);
    call_tool_result(content, is_error)
}

fn call_tool_result(content: Vec<Content>, is_error: bool) -> CallToolResult {
    if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// Build the HTTP client used only for process-local callback traffic. Failure
/// is fatal to bridge startup: falling back to `Client::new()` would silently
/// lose the loopback-only proxy policy and absolute request deadline.
pub fn build_bridge_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("failed to build bounded loopback HTTP client: {error}"))
}

async fn collect_bounded_response_stream<S, B, E>(
    declared_length: Option<u64>,
    mut stream: S,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, String>
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    if declared_length.is_some_and(|length| length > max_bytes as u64) {
        return Err(format!(
            "{label} exceeds the fixed {max_bytes}-byte response limit (declared Content-Length)"
        ));
    }
    let initial_capacity = declared_length
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(max_bytes);
    let mut output = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("failed to read {label}: {error}"))?;
        let bytes = chunk.as_ref();
        if bytes.len() > max_bytes.saturating_sub(output.len()) {
            return Err(format!(
                "{label} exceeds the fixed {max_bytes}-byte response limit"
            ));
        }
        output.extend_from_slice(bytes);
    }
    Ok(output)
}

async fn read_bounded_response_text(
    response: reqwest::Response,
    max_bytes: usize,
    label: &str,
) -> Result<String, String> {
    // With reqwest gzip decoding enabled, `bytes_stream` yields decompressed
    // bytes. `content_length` is only an early rejection hint; the cumulative
    // stream check remains authoritative for chunked bodies, compressed bodies,
    // and maliciously understated metadata.
    let declared_length = response.content_length();
    let bytes = collect_bounded_response_stream(
        declared_length,
        response.bytes_stream(),
        max_bytes,
        label,
    )
    .await?;
    String::from_utf8(bytes).map_err(|error| format!("{label} is not valid UTF-8: {error}"))
}

#[derive(Clone)]
pub struct ScopedBridgeClient<S> {
    inner: Arc<ScopedBridgeInner<S>>,
}

struct ScopedBridgeInner<S> {
    port: u16,
    domain: &'static str,
    log_prefix: &'static str,
    renewal: LoopbackCapabilityRenewalRequest,
    immutable_claims: LoopbackCapabilityClaims<S>,
    access: Mutex<ScopedAccess<S>>,
    http_client: reqwest::Client,
    validate_domain: fn(&LoopbackCapabilityClaims<S>) -> Result<(), LoopbackCapabilityError>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl<S> ScopedBridgeClient<S>
where
    S: Clone + Debug + PartialEq + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    /// Parse the sole capability env and canonicalize it against the parent
    /// process registry before exposing any MCP operation.
    pub async fn from_env(
        env_name: &str,
        domain: &'static str,
        log_prefix: &'static str,
        validate_domain: fn(
            &LoopbackCapabilityClaims<S>,
        ) -> Result<(), LoopbackCapabilityError>,
    ) -> Result<Self, String> {
        let raw = std::env::var(env_name).map_err(|_| format!("missing {env_name}"))?;
        let bootstrap: ScopedMcpChildBootstrap<LoopbackCapabilityClaims<S>> =
            serde_json::from_str(&raw).map_err(|error| format!("invalid {env_name}: {error}"))?;
        Self::from_bootstrap(
            bootstrap,
            domain,
            log_prefix,
            validate_domain,
            Arc::new(unix_time_secs),
        )
        .await
    }

    pub(crate) async fn from_bootstrap(
        bootstrap: ScopedMcpChildBootstrap<LoopbackCapabilityClaims<S>>,
        domain: &'static str,
        log_prefix: &'static str,
        validate_domain: fn(
            &LoopbackCapabilityClaims<S>,
        ) -> Result<(), LoopbackCapabilityError>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, String> {
        if bootstrap.port == 0 {
            return Err("capability bootstrap has invalid loopback port".into());
        }
        bootstrap
            .access
            .claims
            .validate_renewable_shape()
            .map_err(|error| error.to_string())?;
        validate_domain(&bootstrap.access.claims).map_err(|error| error.to_string())?;
        if bootstrap.renewal.lease_id != bootstrap.access.claims.lease_id {
            return Err("capability bootstrap lease mismatch".into());
        }

        let client = Self {
            inner: Arc::new(ScopedBridgeInner {
                port: bootstrap.port,
                domain,
                log_prefix,
                renewal: bootstrap.renewal,
                immutable_claims: bootstrap.access.claims.clone(),
                access: Mutex::new(bootstrap.access),
                http_client: build_bridge_http_client()?,
                validate_domain,
                clock,
            }),
        };

        // Always renew at startup. ACP and Nomi may respawn an MCP process
        // from the original env long after its first access token expired.
        client.ensure_access(true).await?;
        Ok(client)
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// One authenticated POST to a loopback side-route (e.g. the knowledge
    /// server's `/context`). Returns the parsed JSON body on HTTP 200; any
    /// transport/auth/status failure is an `Err` the caller degrades on —
    /// side-routes are best-effort metadata, never tool delivery.
    pub(crate) async fn post_json(
        &self,
        path: &str,
        mut body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let access = self.ensure_access(false).await?;
        inject_session(&mut body, &access.claims);
        let url = format!("http://127.0.0.1:{}{}", self.inner.port, path);
        let response = self
            .inner
            .http_client
            .post(&url)
            .bearer_auth(&access.token)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("loopback POST {path} failed: {error}"))?;
        let status = response.status();
        let text = read_bounded_response_text(
            response,
            MAX_LOOPBACK_CONTROL_RESPONSE_BYTES,
            &format!("loopback POST {path} response"),
        )
        .await?;
        if !status.is_success() {
            return Err(format!("loopback POST {path} -> status={status}"));
        }
        serde_json::from_str(&text)
            .map_err(|error| format!("loopback POST {path} returned invalid JSON: {error}"))
    }

    pub async fn access(&self) -> Result<ScopedAccess<S>, String> {
        self.ensure_access(false).await
    }

    pub async fn access_for(&self, operation: &str) -> Result<ScopedAccess<S>, String> {
        let access = self.ensure_access(false).await?;
        if !access.claims.allows(operation) {
            return Err(format!(
                "operation is outside capability scope: {operation}"
            ));
        }
        Ok(access)
    }

    async fn ensure_access(&self, force: bool) -> Result<ScopedAccess<S>, String> {
        // The mutex is deliberately held across renewal I/O: every concurrent
        // caller observes one refresh and reuses its result (single-flight).
        let mut current = self.inner.access.lock().await;
        let now = (self.inner.clock)();
        if !force
            && current.claims.validate_at(now).is_ok()
            && current.claims.expires_at_unix_secs
                > now.saturating_add(LOOPBACK_CAPABILITY_RENEWAL_MARGIN_SECS)
        {
            return Ok(current.clone());
        }

        let renewed = self.request_renewal().await?;
        self.validate_renewed_access(&renewed, now)?;
        *current = renewed.clone();
        Ok(renewed)
    }

    /// Renew after a request was rejected with this exact access token. If a
    /// concurrent caller already replaced that token while we waited for the
    /// mutex, reuse its fresh access instead of serially renewing again.
    async fn renew_after_unauthorized(
        &self,
        rejected_token: &str,
    ) -> Result<ScopedAccess<S>, String> {
        let mut current = self.inner.access.lock().await;
        if current.token != rejected_token {
            return Ok(current.clone());
        }

        let now = (self.inner.clock)();
        let renewed = self.request_renewal().await?;
        self.validate_renewed_access(&renewed, now)?;
        *current = renewed.clone();
        Ok(renewed)
    }

    fn validate_renewed_access(&self, renewed: &ScopedAccess<S>, now: u64) -> Result<(), String> {
        renewed
            .claims
            .validate_at(now)
            .map_err(|error| format!("invalid renewed capability: {error}"))?;
        (self.inner.validate_domain)(&renewed.claims)
            .map_err(|error| format!("invalid renewed capability scope: {error}"))?;
        if renewed.token.is_empty() || renewed.token.trim() != renewed.token {
            return Err("renewal returned an invalid access token".into());
        }

        let expected = &self.inner.immutable_claims;
        let actual = &renewed.claims;
        if actual.version != expected.version
            || actual.lease_id != expected.lease_id
            || actual.user_id != expected.user_id
            || actual.session != expected.session
            || actual.allowed_tools != expected.allowed_tools
            || actual.scope != expected.scope
        {
            return Err("renewal changed immutable capability authorization".into());
        }
        Ok(())
    }

    async fn request_renewal(&self) -> Result<ScopedAccess<S>, String> {
        let url = format!(
            "http://127.0.0.1:{}{}",
            self.inner.port, LOOPBACK_CAPABILITY_RENEW_PATH
        );
        let delays_ms = [0_u64, 250, 750, 1_500];
        let mut last_error = String::new();
        for (attempt, delay_ms) in delays_ms.into_iter().enumerate() {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            match self
                .inner
                .http_client
                .post(&url)
                .json(&self.inner.renewal)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    let text = match read_bounded_response_text(
                        response,
                        MAX_LOOPBACK_CONTROL_RESPONSE_BYTES,
                        "capability renewal response",
                    )
                    .await
                    {
                        Ok(text) => text,
                        Err(error) => {
                            last_error = error;
                            continue;
                        }
                    };
                    if status.is_success() {
                        return serde_json::from_str(&text).map_err(|error| {
                            format!(
                                "{} renewal returned malformed access: {error}",
                                self.inner.domain
                            )
                        });
                    }
                    last_error = format!(
                        "{} renewal rejected with HTTP {status}",
                        self.inner.domain
                    );
                    if status.is_client_error() {
                        break;
                    }
                }
                Err(error) => {
                    last_error = format!("renewal transport failed: {error:#}");
                }
            }
            eprintln!(
                "[{}] capability renewal retry {}",
                self.inner.log_prefix,
                attempt + 2
            );
        }
        Err(last_error)
    }

    /// Forward one tool call while preserving whether the remote endpoint
    /// returned a tool-level error.  This is the MCP-facing variant: callers
    /// must map [`ForwardToolOutcome::Error`] to `CallToolResult.isError=true`.
    pub(crate) async fn forward_tool_outcome(
        &self,
        operation: &str,
        body: serde_json::Value,
        stringify_non_string_result: bool,
    ) -> ForwardToolOutcome {
        self.forward_tool_outcome_inner(
            operation,
            body,
            stringify_non_string_result,
            ToolDeliveryPolicy::AtLeastOnce {
                idempotency_key: None,
            },
        )
        .await
    }

    /// Forward one tool call with at-most-once delivery: the POST is retried
    /// only while the request provably never reached the server (connection
    /// setup failed). Once it may have been delivered — a timeout, a reset
    /// after send, or a lost response body — the transport error is returned
    /// WITHOUT re-POSTing, because the endpoint may already have committed a
    /// non-idempotent side effect and performs no idempotency-key dedup.
    ///
    /// A 401 still renews the capability and re-POSTs exactly once: the
    /// server rejects unauthorized requests before dispatching any tool, so
    /// that response is proof the side effect was not executed.
    pub(crate) async fn forward_tool_outcome_at_most_once(
        &self,
        operation: &str,
        body: serde_json::Value,
        stringify_non_string_result: bool,
    ) -> ForwardToolOutcome {
        self.forward_tool_outcome_inner(
            operation,
            body,
            stringify_non_string_result,
            ToolDeliveryPolicy::AtMostOnce,
        )
        .await
    }

    /// Forward a tool call with one transport-level business operation key.
    ///
    /// The same exact header value is retained across all four transport
    /// attempts and across the one permitted 401 capability renewal. This is
    /// the only method the Platform Gateway stdio bridge uses.
    pub(crate) async fn forward_tool_outcome_idempotent(
        &self,
        operation: &str,
        body: serde_json::Value,
        stringify_non_string_result: bool,
        idempotency_key: &str,
    ) -> ForwardToolOutcome {
        if !valid_idempotency_key(idempotency_key) {
            return ForwardToolOutcome::Error(
                "Error: invalid idempotency key (expected 1..=128 visible ASCII bytes)"
                    .to_owned(),
            );
        }
        self.forward_tool_outcome_inner(
            operation,
            body,
            stringify_non_string_result,
            ToolDeliveryPolicy::AtLeastOnce {
                idempotency_key: Some(idempotency_key),
            },
        )
        .await
    }

    async fn forward_tool_outcome_inner(
        &self,
        operation: &str,
        mut body: serde_json::Value,
        stringify_non_string_result: bool,
        delivery: ToolDeliveryPolicy<'_>,
    ) -> ForwardToolOutcome {
        let first = match self.access_for(operation).await {
            Ok(access) => access,
            Err(error) => return ForwardToolOutcome::Error(format!("Error: {error}")),
        };
        inject_session(&mut body, &first.claims);
        let first_response = self
            .post_tool_with_retry(&first.token, &body, delivery)
            .await;

        let (status, text) = match first_response {
            Ok(response) if response.0 == reqwest::StatusCode::UNAUTHORIZED => {
                let renewed = match self.renew_after_unauthorized(&first.token).await {
                    Ok(access) => access,
                    Err(error) => {
                        return ForwardToolOutcome::Error(format!(
                            "Error: capability renewal failed: {error}"
                        ));
                    }
                };
                inject_session(&mut body, &renewed.claims);
                match self
                    .post_tool_with_retry(&renewed.token, &body, delivery)
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return ForwardToolOutcome::Error(format!("Error: {error}"));
                    }
                }
            }
            Ok(response) => response,
            Err(error) => return ForwardToolOutcome::Error(format!("Error: {error}")),
        };

        eprintln!(
            "[{}] POST /tool -> status={status}",
            self.inner.log_prefix
        );
        render_tool_response(status, &text, stringify_non_string_result)
    }

    async fn post_tool_with_retry(
        &self,
        token: &str,
        body: &serde_json::Value,
        delivery: ToolDeliveryPolicy<'_>,
    ) -> Result<(reqwest::StatusCode, String), String> {
        let url = format!("http://127.0.0.1:{}/tool", self.inner.port);
        let delays_ms = [0_u64, 250, 750, 1_500];
        let idempotency_key = delivery.idempotency_key();
        let mut last_error = String::new();
        for delay_ms in delays_ms {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let mut request = self
                .inner
                .http_client
                .post(&url)
                .bearer_auth(token)
                .json(body);
            if let Some(idempotency_key) = idempotency_key {
                request = request.header(IDEMPOTENCY_KEY_HEADER, idempotency_key);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    match read_bounded_response_text(
                        response,
                        MAX_LOOPBACK_TOOL_RESPONSE_BYTES,
                        "loopback tool response",
                    )
                    .await
                    {
                        Ok(text) => return Ok((status, text)),
                        Err(error) => {
                            // The server may already have committed the side
                            // effect before the response body is lost. Retry
                            // only with the exact same idempotency key.
                            let error =
                                format!("failed to read response: {error}");
                            if idempotency_key.is_none() {
                                return Err(error);
                            }
                            last_error = error;
                        }
                    }
                }
                Err(error) => {
                    // A connection-setup failure proves the request never left
                    // this process, so even at-most-once delivery may retry it.
                    // Any other send failure (timeout, reset after send) may
                    // have delivered — and possibly executed — the request.
                    let undelivered = error.is_connect();
                    let error = format!("tool transport failed: {error:#}");
                    if matches!(delivery, ToolDeliveryPolicy::AtMostOnce) && !undelivered {
                        return Err(format!(
                            "{error}; the request may already have been executed and was not retried"
                        ));
                    }
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    /// Best-effort child-side teardown. The main runtime/PTY guard is the
    /// independent backstop when the child is killed abruptly.
    pub async fn revoke(&self) {
        let url = format!(
            "http://127.0.0.1:{}{}",
            self.inner.port, LOOPBACK_CAPABILITY_REVOKE_PATH
        );
        let _ = self
            .inner
            .http_client
            .post(url)
            .timeout(Duration::from_secs(2))
            .json(&self.inner.renewal)
            .send()
            .await;
    }
}

fn inject_session<S: Serialize>(body: &mut serde_json::Value, claims: &LoopbackCapabilityClaims<S>) {
    let Some(object) = body.as_object_mut() else {
        *body = serde_json::json!({});
        return inject_session(body, claims);
    };
    object.insert(
        "session".into(),
        serde_json::to_value(claims).expect("validated capability claims serialize"),
    );
}

fn render_tool_response(
    status: reqwest::StatusCode,
    text: &str,
    stringify_non_string_result: bool,
) -> ForwardToolOutcome {
    if !status.is_success() {
        return ForwardToolOutcome::Error(text.to_owned());
    }

    let value = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => value,
        Err(error) => {
            return ForwardToolOutcome::Error(format!(
                "Error: invalid loopback tool response (expected JSON envelope): {error}"
            ));
        }
    };

    // Loopback handlers use an explicit top-level result/error envelope.  Keep
    // this fail-closed: a malformed 2xx response must never turn into a
    // successful MCP result.  `needs_confirmation` is the one explicit control
    // outcome emitted by the gateway permission gate.
    let has_result = value.get("result").is_some();
    let has_error = value.get("error").is_some();
    let is_confirmation = value
        .get("needs_confirmation")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    if has_result && has_error {
        return ForwardToolOutcome::Error(
            "Error: invalid loopback tool response (both `result` and `error` are present)".into(),
        );
    }
    if is_confirmation && (has_result || has_error) {
        return ForwardToolOutcome::Error(
            "Error: invalid loopback tool response (confirmation mixed with result envelope)"
                .into(),
        );
    }
    if let Some(error) = value.get("error") {
        return ForwardToolOutcome::Error(format!("Error: {error}"));
    }
    if let Some(result) = value.get("result") {
        return match result {
            serde_json::Value::String(result) => ForwardToolOutcome::Success(result.clone()),
            _ if stringify_non_string_result => ForwardToolOutcome::Success(
                serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()),
            ),
            _ => ForwardToolOutcome::Success(text.to_owned()),
        };
    }
    if is_confirmation {
        return ForwardToolOutcome::Success(text.to_owned());
    }

    ForwardToolOutcome::Error(
        "Error: invalid loopback tool response (missing `result` or `error`)".into(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use axum::{Json, body::{Body, Bytes}};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use nomifun_common::{
        LOOPBACK_CAPABILITY_TTL_SECS, LoopbackCapabilityIssuer,
        LoopbackSessionBinding,
    };
    use serde::{Deserialize, Serialize};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    use super::*;

    const DOMAIN: &str = "stdio-common-test-v2";

    #[derive(Clone, Copy)]
    enum TestServiceBehavior {
        Pending,
        Panic,
        Success,
    }

    #[derive(Clone, Copy)]
    struct TestStdioService(TestServiceBehavior);

    impl Service<RoleServer> for TestStdioService {
        fn handle_request(
            &self,
            _request: ClientRequest,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<ServerResult, ErrorData>> + Send + '_ {
            let behavior = self.0;
            async move {
                match behavior {
                    TestServiceBehavior::Pending => std::future::pending().await,
                    TestServiceBehavior::Panic => panic!("synthetic service panic"),
                    TestServiceBehavior::Success => {
                        Ok(ServerResult::EmptyResult(rmcp::model::EmptyResult {}))
                    }
                }
            }
        }

        fn handle_notification(
            &self,
            _notification: ClientNotification,
            _context: NotificationContext<RoleServer>,
        ) -> impl Future<Output = Result<(), ErrorData>> + Send + '_ {
            std::future::ready(Ok(()))
        }

        fn get_info(&self) -> ServerInfo {
            ServerInfo::default()
        }
    }

    fn test_ping_request() -> ClientRequest {
        ClientRequest::PingRequest(rmcp::model::PingRequest {
            method: Default::default(),
            extensions: Default::default(),
        })
    }

    #[test]
    fn bounded_stdio_request_budget_rejects_n_plus_one_and_refunds_drop_and_panic() {
        let budget = ProcessRequestBudget::new(MAX_GATEWAY_STDIO_ACTIVE_REQUESTS, 2);
        let mut permits = (0..MAX_GATEWAY_STDIO_ACTIVE_REQUESTS)
            .map(|_| budget.try_acquire_request().expect("within limit"))
            .collect::<Vec<_>>();
        assert_eq!(
            budget.active_requests(),
            MAX_GATEWAY_STDIO_ACTIVE_REQUESTS
        );
        assert!(budget.try_acquire_request().is_none());

        permits.pop();
        assert_eq!(
            budget.active_requests(),
            MAX_GATEWAY_STDIO_ACTIVE_REQUESTS - 1
        );
        permits.push(budget.try_acquire_request().expect("drop refunds slot"));

        let panic_permit = permits.pop().unwrap();
        let unwind = std::panic::catch_unwind(AssertUnwindSafe(move || {
            let _permit = panic_permit;
            panic!("synthetic handler panic");
        }));
        assert!(unwind.is_err());
        assert_eq!(
            budget.active_requests(),
            MAX_GATEWAY_STDIO_ACTIVE_REQUESTS - 1
        );
        drop(permits);
        assert_eq!(budget.active_requests(), 0);
    }

    #[tokio::test]
    async fn bounded_stdio_transport_never_yields_n_plus_one_request_task() {
        let budget = ProcessRequestBudget::new(MAX_GATEWAY_STDIO_ACTIVE_REQUESTS, 2);
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut transport = BoundedStdioTransport::new_with_limits(
            server_read,
            server_write,
            budget.clone(),
            4 * 1024,
            4 * 1024,
        );

        for id in 0..=MAX_GATEWAY_STDIO_ACTIVE_REQUESTS {
            let line = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "ping",
            })
            .to_string();
            client_write.write_all(line.as_bytes()).await.unwrap();
            client_write.write_all(b"\n").await.unwrap();
        }
        client_write.shutdown().await.unwrap();

        let mut admitted = Vec::new();
        for _ in 0..MAX_GATEWAY_STDIO_ACTIVE_REQUESTS {
            admitted.push(
                Transport::<RoleServer>::receive(&mut transport)
                    .await
                    .expect("request within task limit"),
            );
        }
        assert_eq!(
            budget.active_requests(),
            MAX_GATEWAY_STDIO_ACTIVE_REQUESTS
        );
        assert!(
            Transport::<RoleServer>::receive(&mut transport)
                .await
                .is_none()
        );

        let mut response = String::new();
        let mut response_reader = tokio::io::BufReader::new(client_read);
        response_reader.read_line(&mut response).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            response["error"]["data"]["code"],
            "stdio_request_capacity"
        );
        assert_eq!(response["id"], MAX_GATEWAY_STDIO_ACTIVE_REQUESTS);

        drop(admitted);
        drop(transport);
        assert_eq!(budget.active_requests(), 0);
    }

    #[tokio::test]
    async fn bounded_stdio_overlong_unterminated_frame_fails_closed_without_admission() {
        let budget = ProcessRequestBudget::new(2, 2);
        let (server_io, mut client_io) = tokio::io::duplex(1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let mut transport = BoundedStdioTransport::new_with_limits(
            server_read,
            server_write,
            budget.clone(),
            128,
            1024,
        );

        client_io.write_all(&vec![b'x'; 129]).await.unwrap();
        let received = tokio::time::timeout(
            Duration::from_secs(1),
            Transport::<RoleServer>::receive(&mut transport),
        )
        .await
        .expect("decoder must reject before EOF");
        assert!(received.is_none());
        assert_eq!(budget.active_requests(), 0);
    }

    #[test]
    fn bounded_stdio_pending_response_write_keeps_same_request_id_fenced() {
        let budget = ProcessRequestBudget::new(MAX_GATEWAY_STDIO_ACTIVE_REQUESTS, 2);
        let mut transport = BoundedStdioTransport::new_with_limits(
            tokio::io::empty(),
            tokio::io::sink(),
            budget,
            1024,
            1024,
        );
        let request_id = RequestId::Number(7);
        let original = transport
            .admissions
            .try_admit(&request_id)
            .expect("first request admitted");
        original.handoff_to_response();
        let pending_write = Transport::<RoleServer>::send(
            &mut transport,
            ServerJsonRpcMessage::error(
                ErrorData::internal_error("synthetic", None),
                Some(request_id.clone()),
            ),
        );

        assert!(matches!(
            transport.admissions.try_admit(&request_id),
            Err(RequestAdmissionError::Duplicate)
        ));
        drop(pending_write);
        let reused = transport
            .admissions
            .try_admit(&request_id)
            .expect("dropping response future removes exact old fence");
        drop(reused);
        drop(original);
    }

    #[test]
    fn bounded_stdio_cancelled_handler_guard_refunds_exact_registry_slot() {
        let budget = ProcessRequestBudget::new(1, 1);
        let admissions = ActiveRequestRegistry::new(budget.clone());
        let request_id = RequestId::Number(73);
        let guard = admissions
            .try_admit(&request_id)
            .expect("first request admitted");
        assert_eq!(budget.active_requests(), 1);
        assert!(admissions.contains(&request_id));

        // Models the shield/handler future being dropped before it can return a
        // protocol response. The guard removes only its own Arc reservation.
        drop(guard);
        assert_eq!(budget.active_requests(), 0);
        assert!(!admissions.contains(&request_id));

        let reused = admissions
            .try_admit(&request_id)
            .expect("same id is reusable after exact cancellation refund");
        drop(reused);
        assert_eq!(budget.active_requests(), 0);
    }

    #[tokio::test]
    async fn bounded_stdio_panic_shield_deadline_panic_and_child_shutdown_refund() {
        // A small directly-served peer supplies the opaque RequestContext peer
        // handle without going through initialization.
        let (server_io, _client_io) = tokio::io::duplex(1024);
        let peer_transport = BoundedStdioTransport::new_with_limits(
            server_io,
            tokio::io::sink(),
            ProcessRequestBudget::new(1, 1),
            1024,
            1024,
        );
        let running = rmcp::service::serve_directly::<RoleServer, _, _, _, _>(
            TestStdioService(TestServiceBehavior::Success),
            peer_transport,
            None,
        );
        let peer = running.peer().clone();

        for (behavior, expected_code) in [
            (TestServiceBehavior::Pending, "stdio_request_deadline"),
            (TestServiceBehavior::Panic, "stdio_request_panic"),
        ] {
            let budget = ProcessRequestBudget::new(1, 1);
            let admissions = ActiveRequestRegistry::new(budget.clone());
            let request_id = RequestId::Number(if matches!(behavior, TestServiceBehavior::Pending) {
                101
            } else {
                102
            });
            let guard = admissions.try_admit(&request_id).unwrap();
            let mut context = RequestContext::new(request_id.clone(), peer.clone());
            context.extensions.insert(guard);
            let shield = panic_shield_stdio_service_with_timeout(
                TestStdioService(behavior),
                budget.clone(),
                Duration::from_millis(10),
            );
            let error = Service::<RoleServer>::handle_request(
                &shield,
                test_ping_request(),
                context,
            )
            .await
            .expect_err("shield should turn deadline/panic into protocol error");
            assert_eq!(error.data.unwrap()["code"], expected_code);
            assert_eq!(budget.active_requests(), 1, "response fence remains active");
            drop(admissions.begin_completion(&request_id).unwrap());
            assert_eq!(budget.active_requests(), 0);
        }

        let budget = ProcessRequestBudget::new(1, 1);
        let admissions = ActiveRequestRegistry::new(budget.clone());
        let request_id = RequestId::Number(103);
        let guard = admissions.try_admit(&request_id).unwrap();
        let mut context = RequestContext::new(request_id.clone(), peer);
        context.extensions.insert(guard);
        let shield = panic_shield_stdio_service_with_timeout(
            TestStdioService(TestServiceBehavior::Pending),
            budget.clone(),
            Duration::from_secs(30),
        );
        budget.shutdown.cancel();
        let error = Service::<RoleServer>::handle_request(
            &shield,
            test_ping_request(),
            context,
        )
        .await
        .expect_err("child shutdown cancels pending handler");
        assert_eq!(error.data.unwrap()["code"], "stdio_request_cancelled");
        assert_eq!(budget.active_requests(), 0);
        assert!(!admissions.contains(&request_id));
        drop(running);
    }

    #[tokio::test]
    async fn bounded_stdio_session_fuse_exits_after_n_and_refunds_every_permit() {
        let budget = ProcessRequestBudget::new(1, 1);
        let (server_io, client_io) = tokio::io::duplex(16 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut transport = BoundedStdioTransport::new_with_all_limits(
            server_read,
            server_write,
            budget.clone(),
            1024,
            1024,
            3,
            Duration::from_secs(1),
        );
        for id in 1..=4 {
            let line = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "ping",
            })
            .to_string();
            client_write.write_all(line.as_bytes()).await.unwrap();
            client_write.write_all(b"\n").await.unwrap();
        }

        for id in 1..=3 {
            let mut message = Transport::<RoleServer>::receive(&mut transport)
                .await
                .expect("request within child lifetime");
            let guard = match &mut message {
                JsonRpcMessage::Request(request) => request
                    .request
                    .extensions_mut()
                    .remove::<TransportRequestGuard>()
                    .expect("transport guard"),
                other => panic!("unexpected message: {other:?}"),
            };
            guard.handoff_to_response();
            drop(guard);
            drop(message);
            Transport::<RoleServer>::send(
                &mut transport,
                ServerJsonRpcMessage::error(
                    ErrorData::internal_error("synthetic", None),
                    Some(RequestId::Number(id)),
                ),
            )
            .await
            .unwrap();
            assert_eq!(budget.active_requests(), 0);
        }

        assert!(
            Transport::<RoleServer>::receive(&mut transport)
                .await
                .is_none(),
            "N+1 must fail closed instead of spawning another rmcp task"
        );
        assert_eq!(budget.active_requests(), 0);
        assert!(transport.failure.is_failed());

        let mut lines = tokio::io::BufReader::new(client_read).lines();
        let mut last = serde_json::Value::Null;
        for _ in 0..4 {
            last = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        }
        assert_eq!(
            last["error"]["data"]["code"],
            "stdio_session_lifetime_exhausted"
        );
        assert_eq!(last["id"], 4);
    }

    #[tokio::test]
    async fn bounded_stdio_blocked_writer_times_out_and_refunds_response_fence() {
        let budget = ProcessRequestBudget::new(1, 1);
        let (server_write, _blocked_reader) = tokio::io::duplex(64);
        let mut transport = BoundedStdioTransport::new_with_all_limits(
            tokio::io::empty(),
            server_write,
            budget.clone(),
            1024,
            8 * 1024,
            8,
            Duration::from_millis(30),
        );
        let request_id = RequestId::Number(88);
        let guard = transport
            .admissions
            .try_admit(&request_id)
            .expect("request admitted");
        guard.handoff_to_response();
        drop(guard);

        let error = Transport::<RoleServer>::send(
            &mut transport,
            ServerJsonRpcMessage::error(
                ErrorData::internal_error(
                    "synthetic",
                    Some(serde_json::json!({"payload": "x".repeat(4096)})),
                ),
                Some(request_id.clone()),
            ),
        )
        .await
        .expect_err("peer that does not read must hit the absolute write deadline");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(budget.active_requests(), 0);
        assert!(!transport.admissions.contains(&request_id));
        assert!(transport.failure.is_failed());

        let close = tokio::time::timeout(
            Duration::from_millis(100),
            Transport::<RoleServer>::close(&mut transport),
        )
        .await;
        assert!(close.is_ok(), "close must remain bounded after backpressure");
    }

    #[tokio::test]
    async fn bounded_stdio_failure_signal_is_level_triggered_before_receive_waits() {
        let budget = ProcessRequestBudget::new(1, 1);
        let mut transport = BoundedStdioTransport::new_with_limits(
            tokio::io::duplex(64).0,
            tokio::io::sink(),
            budget,
            1024,
            1024,
        );
        transport.failure.fail();
        let received = tokio::time::timeout(
            Duration::from_millis(50),
            Transport::<RoleServer>::receive(&mut transport),
        )
        .await
        .expect("CancellationToken remembers cancellation before waiter registration");
        assert!(received.is_none());
    }

    #[tokio::test]
    async fn bounded_stdio_eof_cancels_pending_handler_guard_without_waiting_for_deadline() {
        let budget = ProcessRequestBudget::new(1, 1);
        let mut transport = BoundedStdioTransport::new_with_limits(
            tokio::io::empty(),
            tokio::io::sink(),
            budget.clone(),
            1024,
            1024,
        );
        let request_id = RequestId::Number(91);
        let guard = transport
            .admissions
            .try_admit(&request_id)
            .expect("pending handler reservation");
        let child_shutdown = budget.shutdown_token();
        let detached_handler = tokio::spawn(async move {
            child_shutdown.cancelled().await;
            drop(guard);
        });

        assert!(
            Transport::<RoleServer>::receive(&mut transport)
                .await
                .is_none()
        );
        tokio::time::timeout(Duration::from_millis(100), detached_handler)
            .await
            .expect("EOF must cancel detached handler immediately")
            .unwrap();
        assert_eq!(budget.active_requests(), 0);
        assert!(!transport.admissions.contains(&request_id));
    }

    #[tokio::test]
    async fn bounded_stdio_lifetime_fuse_cancels_already_pending_handler() {
        let budget = ProcessRequestBudget::new(1, 1);
        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server_io);
        let mut transport = BoundedStdioTransport::new_with_all_limits(
            server_read,
            server_write,
            budget.clone(),
            1024,
            1024,
            1,
            Duration::from_secs(1),
        );
        for id in 1..=2 {
            let line = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "ping",
            })
            .to_string();
            client_io.write_all(line.as_bytes()).await.unwrap();
            client_io.write_all(b"\n").await.unwrap();
        }

        let mut first = Transport::<RoleServer>::receive(&mut transport)
            .await
            .expect("first request admitted");
        let guard = match &mut first {
            JsonRpcMessage::Request(request) => request
                .request
                .extensions_mut()
                .remove::<TransportRequestGuard>()
                .unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        drop(first);
        let child_shutdown = budget.shutdown_token();
        let detached_handler = tokio::spawn(async move {
            child_shutdown.cancelled().await;
            drop(guard);
        });

        assert!(
            Transport::<RoleServer>::receive(&mut transport)
                .await
                .is_none(),
            "N+1 lifetime request closes the child"
        );
        tokio::time::timeout(Duration::from_millis(100), detached_handler)
            .await
            .expect("lifetime fuse must cancel pending handler")
            .unwrap();
        assert_eq!(budget.active_requests(), 0);
        assert!(!transport.admissions.contains(&RequestId::Number(1)));
    }

    #[test]
    fn bounded_browser_task_family_wire_formula_stays_below_half_gibibyte() {
        assert_eq!(MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY, 16);
        assert_eq!(MAX_BROWSER_STDIO_ACTIVE_REQUESTS, 1);
        assert_eq!(MAX_BROWSER_STDIO_INPUT_FRAME_BYTES, 256 * 1024);
        assert_eq!(MAX_BROWSER_STDIO_OUTPUT_FRAME_BYTES, 20 * 1024 * 1024);
        assert_eq!(MAX_BROWSER_TASK_FAMILY_STDIO_WIRE_BYTES, 348_127_376);
        assert!(MAX_BROWSER_TASK_FAMILY_STDIO_WIRE_BYTES < 512 * 1024 * 1024);
    }

    #[tokio::test]
    async fn bounded_response_stream_rejects_declared_chunked_and_falsely_small_lengths() {
        let oversized = vec![b'x'; 17];
        let declared = collect_bounded_response_stream(
            Some(17),
            futures_util::stream::iter([Ok::<_, io::Error>(oversized.clone())]),
            16,
            "declared body",
        )
        .await
        .expect_err("large Content-Length must fail before accumulation");
        assert!(declared.contains("declared Content-Length"));

        let chunked = collect_bounded_response_stream(
            None,
            futures_util::stream::iter([
                Ok::<_, io::Error>(vec![b'a'; 8]),
                Ok::<_, io::Error>(vec![b'b'; 9]),
            ]),
            16,
            "chunked body",
        )
        .await
        .expect_err("chunked cumulative bytes must be capped");
        assert!(chunked.contains("fixed 16-byte"));

        let false_small = collect_bounded_response_stream(
            Some(1),
            futures_util::stream::iter([
                Ok::<_, io::Error>(vec![b'a'; 8]),
                Ok::<_, io::Error>(vec![b'b'; 9]),
            ]),
            16,
            "false-small body",
        )
        .await
        .expect_err("stream count, not claimed metadata, is authoritative");
        assert!(false_small.contains("fixed 16-byte"));
    }

    #[test]
    fn bounded_stdio_output_encoder_aborts_without_retaining_partial_frame() {
        let mut codec = BoundedOutboundCodec::<serde_json::Value>::new(16);
        let mut output = BytesMut::from(&b"existing"[..]);
        assert!(
            codec
                .encode(
                    serde_json::json!({"payload": "x".repeat(64)}),
                    &mut output
                )
                .is_err()
        );
        assert_eq!(&output[..], b"existing");
    }

    #[test]
    fn response_renderer_preserves_structured_gateway_error() {
        let outcome = render_tool_response(
            reqwest::StatusCode::OK,
            r#"{"error":"invalid arguments for this tool: missing field `kb_id`"}"#,
            true,
        );
        assert!(matches!(outcome, ForwardToolOutcome::Error(text) if text.contains("kb_id")));
    }

    #[test]
    fn response_renderer_does_not_guess_errors_from_text() {
        let outcome = render_tool_response(
            reqwest::StatusCode::OK,
            r#"{"result":"Error: this is ordinary successful tool output"}"#,
            true,
        );
        assert_eq!(
            outcome,
            ForwardToolOutcome::Success("Error: this is ordinary successful tool output".into())
        );

        let nested = render_tool_response(
            reqwest::StatusCode::OK,
            r#"{"result":{"error":"a payload field, not the gateway envelope"}}"#,
            true,
        );
        assert!(matches!(nested, ForwardToolOutcome::Success(_)));
    }

    #[test]
    fn response_renderer_marks_non_success_http_status_as_error() {
        let outcome = render_tool_response(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "upstream unavailable",
            true,
        );
        assert_eq!(
            outcome,
            ForwardToolOutcome::Error("upstream unavailable".into())
        );
    }

    #[test]
    fn response_renderer_rejects_malformed_success_envelopes() {
        for text in ["not json", r#"{"unexpected":true}"#, "null"] {
            let outcome = render_tool_response(reqwest::StatusCode::OK, text, true);
            assert!(
                matches!(outcome, ForwardToolOutcome::Error(ref message) if message.contains("invalid loopback tool response")),
                "unexpected outcome for {text:?}: {outcome:?}"
            );
        }

        let ambiguous = render_tool_response(
            reqwest::StatusCode::OK,
            r#"{"result":"ok","error":"failed"}"#,
            true,
        );
        assert!(matches!(ambiguous, ForwardToolOutcome::Error(message) if message.contains("both `result` and `error`")));

        let mixed_confirmation = render_tool_response(
            reqwest::StatusCode::OK,
            r#"{"result":"ok","needs_confirmation":true}"#,
            true,
        );
        assert!(matches!(mixed_confirmation, ForwardToolOutcome::Error(message) if message.contains("confirmation mixed")));
    }

    #[test]
    fn response_renderer_accepts_explicit_confirmation_outcome() {
        let text = r#"{"needs_confirmation":true,"tool":"nomi_delete"}"#;
        assert_eq!(
            render_tool_response(reqwest::StatusCode::OK, text, true),
            ForwardToolOutcome::Success(text.into())
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestScope {
        resource: String,
    }

    #[derive(Clone)]
    struct TestState {
        issuer: Arc<LoopbackCapabilityIssuer>,
        now: Arc<AtomicU64>,
        renew_count: Arc<AtomicUsize>,
        tool_count: Arc<AtomicUsize>,
        tamper_scope: Arc<AtomicBool>,
        reject_tools: Arc<AtomicBool>,
        fail_response_body_once: Arc<AtomicBool>,
        idempotency_headers: Arc<StdMutex<Vec<Vec<String>>>>,
    }

    fn validate_test_claims(
        claims: &LoopbackCapabilityClaims<TestScope>,
    ) -> Result<(), LoopbackCapabilityError> {
        claims.validate_renewable_shape()?;
        if claims.scope.resource.trim().is_empty() {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        Ok(())
    }

    fn bootstrap(
        issuer: &Arc<LoopbackCapabilityIssuer>,
        port: u16,
        now: u64,
    ) -> ScopedMcpChildBootstrap<LoopbackCapabilityClaims<TestScope>> {
        let claims = LoopbackCapabilityClaims::issue_at(
            "0190f5fe-7c00-7a00-8000-000000000001",
            LoopbackSessionBinding::conversation("0190f5fe-7c00-7a00-8000-000000000001"),
            ["tools/call", "tools/list"],
            TestScope {
                resource: "alpha".into(),
            },
            now,
            LOOPBACK_CAPABILITY_TTL_SECS,
        )
        .unwrap();
        let (token, renewal_proof) = issuer.activate(DOMAIN, &claims).unwrap();
        ScopedMcpChildBootstrap {
            port,
            renewal: LoopbackCapabilityRenewalRequest {
                lease_id: claims.lease_id.clone(),
                renewal_proof,
            },
            access: LoopbackCapabilityAccess { token, claims },
        }
    }

    async fn renew_handler(
        State(state): State<TestState>,
        Json(request): Json<LoopbackCapabilityRenewalRequest>,
    ) -> impl IntoResponse {
        state.renew_count.fetch_add(1, Ordering::SeqCst);
        match state.issuer.renew_at::<TestScope>(
            DOMAIN,
            &request,
            state.now.load(Ordering::SeqCst),
        ) {
            Ok(mut access) => {
                if state.tamper_scope.load(Ordering::SeqCst) {
                    access.claims.scope.resource = "beta".into();
                }
                (StatusCode::OK, Json(serde_json::json!(access))).into_response()
            }
            Err(_) => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response(),
        }
    }

    async fn tool_handler(
        State(state): State<TestState>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        state.tool_count.fetch_add(1, Ordering::SeqCst);
        state.idempotency_headers.lock().unwrap().push(
            headers
                .get_all(IDEMPOTENCY_KEY_HEADER)
                .iter()
                .map(|value| value.to_str().unwrap_or("<invalid>").to_owned())
                .collect(),
        );
        if state.reject_tools.load(Ordering::SeqCst) {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response()
        } else if state
            .fail_response_body_once
            .swap(false, Ordering::SeqCst)
        {
            let body = Body::from_stream(futures_util::stream::iter([
                Ok::<Bytes, std::io::Error>(Bytes::from_static(
                    br#"{"result":"committed"}"#,
                )),
                Err(std::io::Error::other(
                    "simulated response loss after commit",
                )),
            ]));
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .body(body)
                .unwrap()
        } else {
            (
                StatusCode::OK,
                Json(serde_json::json!({"result": "ok"})),
            )
                .into_response()
        }
    }

    async fn spawn_server(
        state: TestState,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new()
            .route(LOOPBACK_CAPABILITY_RENEW_PATH, axum::routing::post(renew_handler))
            .route("/tool", axum::routing::post(tool_handler))
            .with_state(state);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (port, handle)
    }

    fn state(now: u64) -> TestState {
        TestState {
            issuer: Arc::new(LoopbackCapabilityIssuer::random().unwrap()),
            now: Arc::new(AtomicU64::new(now)),
            renew_count: Arc::new(AtomicUsize::new(0)),
            tool_count: Arc::new(AtomicUsize::new(0)),
            tamper_scope: Arc::new(AtomicBool::new(false)),
            reject_tools: Arc::new(AtomicBool::new(false)),
            fail_response_body_once: Arc::new(AtomicBool::new(false)),
            idempotency_headers: Arc::new(StdMutex::new(Vec::new())),
        }
    }

    #[tokio::test]
    async fn expired_original_env_renews_on_start_and_refresh_is_single_flight() {
        let now = unix_time_secs();
        let state = state(now);
        let (port, server) = spawn_server(state.clone()).await;
        let mut bootstrap = bootstrap(&state.issuer, port, now);
        // Simulate an ACP/Nomi MCP respawn from the original env after access
        // expiry. Renewal proof remains bound to the active process lease.
        bootstrap.access.claims.issued_at_unix_secs = now.saturating_sub(61);
        bootstrap.access.claims.expires_at_unix_secs = now.saturating_sub(1);

        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .expect("expired bootstrap must canonical-renew");
        assert_eq!(state.renew_count.load(Ordering::SeqCst), 1);

        let first = client.access().await.unwrap();
        state.now.store(
            first
                .claims
                .expires_at_unix_secs
                .saturating_sub(LOOPBACK_CAPABILITY_RENEWAL_MARGIN_SECS)
                .saturating_add(1),
            Ordering::SeqCst,
        );
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let client = client.clone();
            tasks.push(tokio::spawn(async move { client.access().await.unwrap() }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(
            state.renew_count.load(Ordering::SeqCst),
            2,
            "all concurrent refreshes must share one renewal"
        );
        server.abort();
    }

    #[tokio::test]
    async fn renewal_rejects_server_response_that_changes_full_immutable_scope() {
        let now = unix_time_secs();
        let state = state(now);
        state.tamper_scope.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();

        let error = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .err()
        .expect("tampered renewal must fail closed");
        assert!(error.contains("immutable capability authorization"));
        server.abort();
    }

    #[tokio::test]
    async fn unauthorized_tool_response_forces_one_renewal_and_one_retry_only() {
        let now = unix_time_secs();
        let state = state(now);
        state.reject_tools.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        let result = client
            .forward_tool_outcome(
                "tools/call",
                serde_json::json!({"tool": "demo", "args": {}}),
                false,
            )
            .await;
        assert!(
            matches!(result, ForwardToolOutcome::Error(text) if text.contains("unauthorized"))
        );
        assert_eq!(state.renew_count.load(Ordering::SeqCst), 2);
        assert_eq!(state.tool_count.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn idempotency_key_is_unchanged_across_unauthorized_renewal() {
        let now = unix_time_secs();
        let state = state(now);
        state.reject_tools.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();
        let key = "gateway-tool-v1-stable-renewal";

        let result = client
            .forward_tool_outcome_idempotent(
                "tools/call",
                serde_json::json!({"tool": "demo", "args": {}}),
                false,
                key,
            )
            .await;

        assert!(matches!(result, ForwardToolOutcome::Error(_)));
        assert_eq!(state.tool_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            *state.idempotency_headers.lock().unwrap(),
            vec![vec![key.to_owned()], vec![key.to_owned()]]
        );
        server.abort();
    }

    #[tokio::test]
    async fn at_most_once_never_reposts_after_possible_delivery() {
        let now = unix_time_secs();
        let state = state(now);
        // The response stream aborts after the server has already committed —
        // exactly the window where a re-POST would double-execute a click.
        state.fail_response_body_once.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        let result = client
            .forward_tool_outcome_at_most_once(
                "tools/call",
                serde_json::json!({"tool": "click", "args": {"ref": "f1e2"}}),
                false,
            )
            .await;

        // Depending on where the abort lands, the loss surfaces as a body-read
        // failure or as a post-send transport failure; both count as possible
        // delivery and must not retry.
        assert!(
            matches!(
                result,
                ForwardToolOutcome::Error(ref message)
                    if message.contains("failed to read response")
                        || message.contains("was not retried")
            ),
            "unexpected outcome: {result:?}"
        );
        assert_eq!(
            state.tool_count.load(Ordering::SeqCst),
            1,
            "a possibly-executed browser action must never be re-POSTed"
        );
        server.abort();
    }

    #[tokio::test]
    async fn at_most_once_retries_undelivered_connection_failures() {
        let now = unix_time_secs();
        let state = state(now);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        // Take the server down (connection refused: the request provably never
        // leaves the client), then bring it back on the same port before the
        // client's third transport attempt (delays are 0/250/750/1500 ms).
        server.abort();
        let restart_state = state.clone();
        let restart = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let listener = loop {
                match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => break listener,
                    Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
                }
            };
            let app = axum::Router::new()
                .route(
                    LOOPBACK_CAPABILITY_RENEW_PATH,
                    axum::routing::post(renew_handler),
                )
                .route("/tool", axum::routing::post(tool_handler))
                .with_state(restart_state);
            axum::serve(listener, app).await.unwrap();
        });

        let result = client
            .forward_tool_outcome_at_most_once(
                "tools/call",
                serde_json::json!({"tool": "observe", "args": {}}),
                false,
            )
            .await;

        assert_eq!(result, ForwardToolOutcome::Success("ok".into()));
        assert_eq!(
            state.tool_count.load(Ordering::SeqCst),
            1,
            "the refused attempts never reached the server"
        );
        restart.abort();
    }

    #[tokio::test]
    async fn at_most_once_still_renews_once_after_unauthorized() {
        let now = unix_time_secs();
        let state = state(now);
        state.reject_tools.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        let result = client
            .forward_tool_outcome_at_most_once(
                "tools/call",
                serde_json::json!({"tool": "click", "args": {"ref": "f1e2"}}),
                false,
            )
            .await;

        // A 401 is proof the tool was not dispatched, so the single renewed
        // re-POST is compatible with at-most-once execution.
        assert!(matches!(result, ForwardToolOutcome::Error(text) if text.contains("unauthorized")));
        assert_eq!(state.renew_count.load(Ordering::SeqCst), 2);
        assert_eq!(state.tool_count.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn response_body_loss_retries_with_the_same_single_idempotency_header() {
        let now = unix_time_secs();
        let state = state(now);
        state.fail_response_body_once.store(true, Ordering::SeqCst);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();
        let key = "gateway-tool-v1-stable-response-loss";

        let result = client
            .forward_tool_outcome_idempotent(
                "tools/call",
                serde_json::json!({"tool": "demo", "args": {}}),
                false,
                key,
            )
            .await;

        assert_eq!(result, ForwardToolOutcome::Success("ok".into()));
        assert_eq!(state.tool_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            *state.idempotency_headers.lock().unwrap(),
            vec![vec![key.to_owned()], vec![key.to_owned()]]
        );
        server.abort();
    }

    #[tokio::test]
    async fn invalid_idempotency_key_fails_before_any_http_attempt() {
        let now = unix_time_secs();
        let state = state(now);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        let result = client
            .forward_tool_outcome_idempotent(
                "tools/call",
                serde_json::json!({"tool": "demo", "args": {}}),
                false,
                "contains space",
            )
            .await;

        assert!(matches!(result, ForwardToolOutcome::Error(message) if message.contains("invalid idempotency key")));
        assert_eq!(state.tool_count.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn concurrent_unauthorized_requests_share_one_forced_renewal() {
        let now = unix_time_secs();
        let state = state(now);
        let (port, server) = spawn_server(state.clone()).await;
        let bootstrap = bootstrap(&state.issuer, port, now);
        let clock_state = state.now.clone();
        let client = ScopedBridgeClient::from_bootstrap(
            bootstrap,
            DOMAIN,
            "test-bridge",
            validate_test_claims,
            Arc::new(move || clock_state.load(Ordering::SeqCst)),
        )
        .await
        .unwrap();

        let rejected_token = client.access().await.unwrap().token;
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let client = client.clone();
            let rejected_token = rejected_token.clone();
            tasks.push(tokio::spawn(async move {
                client
                    .renew_after_unauthorized(&rejected_token)
                    .await
                    .unwrap()
                    .token
            }));
        }

        let mut renewed_tokens = Vec::new();
        for task in tasks {
            renewed_tokens.push(task.await.unwrap());
        }
        assert!(renewed_tokens.iter().all(|token| token != &rejected_token));
        assert!(renewed_tokens.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            state.renew_count.load(Ordering::SeqCst),
            2,
            "startup renewal plus one shared forced renewal"
        );
        server.abort();
    }
}
