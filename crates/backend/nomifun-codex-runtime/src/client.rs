use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
use std::time::Duration;

use nomifun_agent_contracts::{
    FullAutoExecutionWire, IdempotencyKey, NativeActionStartAck, RuntimeBindingContract,
    RuntimeBindingId, RuntimeCommand, RuntimeHelloPayload,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::credential::{
    CredentialHandleDescriptor, InheritedHandleCredential, PreparedCredentialChannel,
    RuntimeHelloRequest,
};
use crate::error::RuntimeError;
use crate::native_action::{
    AckedNativeAction, RuntimeIngressPort, validate_runtime_event_ack,
};
use crate::protocol::{
    RequestId, RpcErrorObject, RuntimeSessionDisposeAck, ServerFrame, StableRpcMethod,
    command_params, decode_open_result, decode_server_frame, encode_request, encode_response,
};
use crate::release::{RUNTIME_HELLO_METHOD, RuntimeHelloExpectation};

type BoxWriter = Pin<Box<dyn AsyncWrite + Send + Unpin>>;
type PendingResult = Result<Value, RpcErrorObject>;
type PendingMap = Arc<StdMutex<HashMap<RequestId, oneshot::Sender<PendingResult>>>>;

#[derive(Clone, Copy, Debug)]
pub struct ClientLimits {
    pub max_frame_bytes: usize,
    pub max_in_flight_requests: usize,
    pub inbound_queue_capacity: usize,
    pub hello_timeout: Duration,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 1024 * 1024,
            max_in_flight_requests: 64,
            inbound_queue_capacity: 64,
            hello_timeout: Duration::from_secs(10),
        }
    }
}

pub struct CodexRuntimeClient {
    writer: Mutex<BoxWriter>,
    pending: PendingMap,
    inbound: Mutex<mpsc::Receiver<ServerFrame>>,
    in_flight: Arc<Semaphore>,
    next_id: AtomicI64,
    closed: AtomicBool,
    fatal_reason: StdMutex<Option<String>>,
    cancellation: CancellationToken,
    reader_task: StdMutex<Option<JoinHandle<()>>>,
    hello: StdMutex<Option<RuntimeHelloPayload>>,
    native_action_acks:
        StdMutex<HashMap<(RuntimeBindingId, IdempotencyKey), NativeActionStartAck>>,
}

impl std::fmt::Debug for CodexRuntimeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodexRuntimeClient")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .field("hello", &lock(&self.hello).as_ref())
            .finish_non_exhaustive()
    }
}

impl CodexRuntimeClient {
    pub async fn connect<R, W>(
        reader: R,
        writer: W,
        credential_channel: PreparedCredentialChannel,
        credential_handle: CredentialHandleDescriptor,
        credential: InheritedHandleCredential,
        expectation: RuntimeHelloExpectation,
        limits: ClientLimits,
    ) -> Result<Arc<Self>, RuntimeError>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        limits.validate()?;
        let client = Self::start(reader, writer, limits);
        let hello_params = serde_json::to_value(RuntimeHelloRequest::new(credential_handle))?;
        let exchange = async {
            tokio::try_join!(
                client.call_raw(RUNTIME_HELLO_METHOD, hello_params),
                credential_channel.transmit_and_close(credential)
            )
        };
        let (hello, ()) = match tokio::time::timeout(limits.hello_timeout, exchange).await {
            Ok(Ok(exchange)) => exchange,
            Ok(Err(error)) => {
                client.close().await;
                return Err(error);
            }
            Err(_) => {
                client.close().await;
                return Err(RuntimeError::Timeout("runtime hello".to_owned()));
            }
        };
        let hello = match serde_json::from_value::<RuntimeHelloPayload>(hello) {
            Ok(hello) => hello,
            Err(error) => {
                client.close().await;
                return Err(RuntimeError::Json(error));
            }
        };
        if hello.full_auto != FullAutoExecutionWire::fixed() {
            client.close().await;
            return Err(RuntimeError::HelloRejected(
                "runtime hello does not advertise fixed FullAuto".to_owned(),
            ));
        }
        if let Err(error) = expectation.validate(&hello) {
            client.close().await;
            return Err(error);
        }
        *lock(&client.hello) = Some(hello);
        Ok(client)
    }

    pub fn hello(&self) -> Option<RuntimeHelloPayload> {
        lock(&self.hello).clone()
    }

    pub async fn command<T>(&self, command: &RuntimeCommand) -> Result<T, RuntimeError>
    where
        T: DeserializeOwned,
    {
        if let RuntimeCommand::Create(params) = command
            && params.full_auto != FullAutoExecutionWire::fixed()
        {
            return Err(RuntimeError::Protocol(
                "create attempted to override fixed FullAuto".to_owned(),
            ));
        }
        let method = StableRpcMethod::from(command);
        let result = self
            .call_raw(method.as_str(), command_params(command)?)
            .await?;
        serde_json::from_value(result).map_err(RuntimeError::Json)
    }

    pub async fn open(
        &self,
        command: &RuntimeCommand,
    ) -> Result<RuntimeBindingContract, RuntimeError> {
        match command {
            RuntimeCommand::Create(_) | RuntimeCommand::Resume(_) | RuntimeCommand::Fork(_) => {
                let value = self
                    .call_raw(
                        StableRpcMethod::from(command).as_str(),
                        command_params(command)?,
                    )
                    .await?;
                decode_open_result(value)
            }
            _ => Err(RuntimeError::Protocol(
                "open accepts only create, resume, or fork".to_owned(),
            )),
        }
    }

    pub async fn dispose(
        &self,
        command: &RuntimeCommand,
    ) -> Result<RuntimeSessionDisposeAck, RuntimeError> {
        if !matches!(command, RuntimeCommand::SessionDispose(_)) {
            return Err(RuntimeError::Protocol(
                "dispose requires session_dispose params".to_owned(),
            ));
        }
        self.command(command).await
    }

    pub async fn serve_ingress(
        self: &Arc<Self>,
        port: Arc<dyn RuntimeIngressPort>,
        cancellation: CancellationToken,
    ) -> Result<(), RuntimeError> {
        loop {
            let frame = tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                frame = self.next_inbound() => frame,
            };
            let Some(frame) = frame else {
                return Err(RuntimeError::Protocol(
                    self.fatal_reason()
                        .unwrap_or_else(|| "runtime inbound stream closed".to_owned()),
                ));
            };

            match frame {
                ServerFrame::RuntimeEvent { id, event } => {
                    let result = port.append_runtime_event(event.clone()).await;
                    match result {
                        Ok(ack) => {
                            if let Err(error) = validate_runtime_event_ack(&event, &ack) {
                                self.respond(id, Err(host_rejection(&error))).await?;
                                return Err(error);
                            }
                            self.respond(id, Ok(serde_json::to_value(ack)?)).await?;
                        }
                        Err(error) => {
                            self.respond(id, Err(host_rejection(&error))).await?;
                            return Err(error);
                        }
                    }
                }
                ServerFrame::NativeActionStart { id, start } => {
                    let key = (
                        start.runtime_binding_id.clone(),
                        start.idempotency_key.clone(),
                    );
                    let cached_ack = { lock(&self.native_action_acks).get(&key).cloned() };
                    if let Some(ack) = cached_ack {
                        match AckedNativeAction::after_durable_commit(start, ack.clone()) {
                            Ok(permit) => {
                                self.respond(id, Ok(serde_json::to_value(permit.ack())?))
                                    .await?;
                                continue;
                            }
                            Err(_) => {
                                let error = RuntimeError::NativeActionAlreadyCommitted(
                                    key.1.as_ref().to_owned(),
                                );
                                self.respond(id, Err(host_rejection(&error))).await?;
                                return Err(error);
                            }
                        }
                    }

                    let result = port.commit_native_action_start(start.clone()).await;
                    match result {
                        Ok(ack) => {
                            let permit = match AckedNativeAction::after_durable_commit(
                                start,
                                ack.clone(),
                            ) {
                                Ok(permit) => permit,
                                Err(error) => {
                                    self.respond(id, Err(host_rejection(&error))).await?;
                                    return Err(error);
                                }
                            };
                            lock(&self.native_action_acks).insert(key, ack);
                            self.respond(id, Ok(serde_json::to_value(permit.ack())?))
                                .await?;
                        }
                        Err(error) => {
                            self.respond(id, Err(host_rejection(&error))).await?;
                            return Err(error);
                        }
                    }
                }
                ServerFrame::Response { .. } => {
                    return Err(RuntimeError::Protocol(
                        "response was routed to the inbound request queue".to_owned(),
                    ));
                }
            }
        }
    }

    pub async fn close(&self) {
        let first_close = !self.closed.swap(true, Ordering::AcqRel);
        self.cancellation.cancel();
        {
            let mut writer = self.writer.lock().await;
            let _ = writer.as_mut().shutdown().await;
        }
        if first_close {
            fail_all_pending(
                &self.pending,
                "runtime client closed before the response arrived",
            );
        }
        let reader_task = { lock(&self.reader_task).take() };
        if let Some(task) = reader_task {
            task.abort();
            let _ = task.await;
        }
    }

    pub fn fatal_reason(&self) -> Option<String> {
        lock(&self.fatal_reason).clone()
    }

    fn start<R, W>(reader: R, writer: W, limits: ClientLimits) -> Arc<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (inbound_tx, inbound_rx) = mpsc::channel(limits.inbound_queue_capacity);
        let client = Arc::new(Self {
            writer: Mutex::new(Box::pin(writer)),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            inbound: Mutex::new(inbound_rx),
            in_flight: Arc::new(Semaphore::new(limits.max_in_flight_requests)),
            next_id: AtomicI64::new(1),
            closed: AtomicBool::new(false),
            fatal_reason: StdMutex::new(None),
            cancellation: CancellationToken::new(),
            reader_task: StdMutex::new(None),
            hello: StdMutex::new(None),
            native_action_acks: StdMutex::new(HashMap::new()),
        });
        let task_client = Arc::clone(&client);
        let task = tokio::spawn(async move {
            task_client
                .reader_loop(reader, inbound_tx, limits.max_frame_bytes)
                .await;
        });
        *lock(&client.reader_task) = Some(task);
        client
    }

    async fn call_raw(&self, method: &str, params: Value) -> Result<Value, RuntimeError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RuntimeError::Protocol(
                self.fatal_reason()
                    .unwrap_or_else(|| "runtime client is closed".to_owned()),
            ));
        }
        let _permit = self
            .in_flight
            .acquire()
            .await
            .map_err(|_| RuntimeError::Protocol("runtime client is closed".to_owned()))?;
        let id = RequestId::Integer(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(id.clone(), sender);
        let registration = PendingRegistration {
            id: id.clone(),
            pending: Arc::clone(&self.pending),
        };

        let encoded = encode_request(id, method, params)?;
        {
            let mut writer = self.writer.lock().await;
            writer.as_mut().write_all(&encoded).await?;
            writer.as_mut().flush().await?;
        }

        let result = receiver.await.map_err(|_| {
            RuntimeError::Protocol(
                self.fatal_reason()
                    .unwrap_or_else(|| "runtime response channel closed".to_owned()),
            )
        })?;
        drop(registration);
        match result {
            Ok(value) => Ok(value),
            Err(error) => Err(RuntimeError::Rpc {
                code: error.code,
                message: error.message,
                data: error.data,
            }),
        }
    }

    async fn respond(
        &self,
        id: RequestId,
        result: PendingResult,
    ) -> Result<(), RuntimeError> {
        let encoded = encode_response(id, result)?;
        let mut writer = self.writer.lock().await;
        writer.as_mut().write_all(&encoded).await?;
        writer.as_mut().flush().await?;
        Ok(())
    }

    async fn next_inbound(&self) -> Option<ServerFrame> {
        self.inbound.lock().await.recv().await
    }

    async fn reader_loop<R>(
        &self,
        reader: R,
        inbound: mpsc::Sender<ServerFrame>,
        max_frame_bytes: usize,
    ) where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let mut reader = BufReader::new(reader);
        loop {
            let mut frame = Vec::new();
            let frame_limit = u64::try_from(max_frame_bytes.saturating_add(1))
                .unwrap_or(u64::MAX);
            let mut limited = (&mut reader).take(frame_limit);
            let read = tokio::select! {
                _ = self.cancellation.cancelled() => return,
                read = limited.read_until(b'\n', &mut frame) => read,
            };
            match read {
                Ok(0) => {
                    self.fail_fatal("runtime stdio reached EOF".to_owned());
                    return;
                }
                Ok(_) if frame.len() > max_frame_bytes => {
                    self.fail_fatal(format!(
                        "runtime frame exceeds {max_frame_bytes} bytes"
                    ));
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    self.fail_fatal(format!("runtime stdio read failed: {error}"));
                    return;
                }
            }
            while matches!(frame.last(), Some(b'\n' | b'\r')) {
                frame.pop();
            }
            if frame.is_empty() {
                self.fail_fatal("runtime emitted an empty JSONL frame".to_owned());
                return;
            }

            let decoded = match decode_server_frame(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    self.fail_fatal(error.to_string());
                    return;
                }
            };
            match decoded {
                ServerFrame::Response { id, result } => {
                    let sender = lock(&self.pending).remove(&id);
                    let Some(sender) = sender else {
                        self.fail_fatal(format!(
                            "runtime returned an unknown or duplicate response id {id:?}"
                        ));
                        return;
                    };
                    let _ = sender.send(result);
                }
                request => {
                    if inbound.send(request).await.is_err() {
                        self.fail_fatal(
                            "runtime inbound consumer closed before protocol shutdown".to_owned(),
                        );
                        return;
                    }
                }
            }
        }
    }

    fn fail_fatal(&self, reason: String) {
        self.closed.store(true, Ordering::Release);
        self.cancellation.cancel();
        let mut fatal = lock(&self.fatal_reason);
        if fatal.is_none() {
            *fatal = Some(reason.clone());
        }
        drop(fatal);
        fail_all_pending(&self.pending, &reason);
    }
}

impl ClientLimits {
    fn validate(self) -> Result<(), RuntimeError> {
        if self.max_frame_bytes == 0
            || self.max_in_flight_requests == 0
            || self.inbound_queue_capacity == 0
            || self.hello_timeout.is_zero()
        {
            return Err(RuntimeError::Protocol(
                "runtime client limits must all be non-zero".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Drop for CodexRuntimeClient {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

struct PendingRegistration {
    id: RequestId,
    pending: PendingMap,
}

impl Drop for PendingRegistration {
    fn drop(&mut self) {
        lock(&self.pending).remove(&self.id);
    }
}

fn fail_all_pending(pending: &PendingMap, reason: &str) {
    let senders = lock(pending)
        .drain()
        .map(|(_, sender)| sender)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ = sender.send(Err(RpcErrorObject {
            code: -32099,
            message: reason.to_owned(),
            data: None,
        }));
    }
}

fn host_rejection(error: &RuntimeError) -> RpcErrorObject {
    RpcErrorObject {
        code: -32040,
        message: error.to_string(),
        data: None,
    }
}

fn lock<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;
    use nomifun_agent_contracts::{
        NativeActionStart, NativeActionStartAck, NativeActionStartAckExchange,
        RuntimeEventWireAck, RuntimeEventWireEnvelope,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::credential::{CREDENTIAL_FRAME_MAGIC, PreparedCredentialChannel};

    struct CommitPort {
        exchange: NativeActionStartAckExchange,
        commit_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl RuntimeIngressPort for CommitPort {
        async fn append_runtime_event(
            &self,
            _event: RuntimeEventWireEnvelope,
        ) -> Result<RuntimeEventWireAck, RuntimeError> {
            unreachable!("test only sends a native action")
        }

        async fn commit_native_action_start(
            &self,
            start: NativeActionStart,
        ) -> Result<NativeActionStartAck, RuntimeError> {
            assert_eq!(start, self.exchange.start);
            self.commit_count.fetch_add(1, Ordering::AcqRel);
            Ok(self.exchange.ack.clone())
        }
    }

    #[tokio::test]
    async fn hello_uses_separate_credential_handle_and_exact_fixture() {
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let (credential_reader, credential_writer) = tokio::io::duplex(1024);
        let channel = PreparedCredentialChannel::from_test_writer(credential_writer);
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/hello-rpc-allowlist.json"
        );
        let expected_payload: RuntimeHelloPayload = serde_json::from_str(fixture).unwrap();
        let expectation = RuntimeHelloExpectation::from_payload(expected_payload.clone());

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["method"], RUNTIME_HELLO_METHOD);
            assert_eq!(
                request["params"]["credential_protocol"],
                "nomifun-inherited-handle-v1"
            );

            let response = serde_json::json!({
                "id": request["id"],
                "result": expected_payload,
            });
            server_writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
            server_writer.flush().await.unwrap();
        });

        let credential = InheritedHandleCredential::new(b"secret".to_vec()).unwrap();
        let client = CodexRuntimeClient::connect(
            client_reader,
            client_writer,
            channel,
            CredentialHandleDescriptor::WindowsHandle { value: 77 },
            credential,
            expectation,
            ClientLimits::default(),
        )
        .await
        .unwrap();

        let mut credential_reader = credential_reader;
        let mut credential_bytes = Vec::new();
        credential_reader
            .read_to_end(&mut credential_bytes)
            .await
            .unwrap();
        assert!(credential_bytes.starts_with(CREDENTIAL_FRAME_MAGIC));
        assert!(!format!("{client:?}").contains("secret"));
        client.close().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn native_action_response_is_written_only_after_durable_commit() {
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (mut server_reader, mut server_writer) = tokio::io::split(server_io);
        let client = CodexRuntimeClient::start(
            client_reader,
            client_writer,
            ClientLimits::default(),
        );
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/native-action-start-ack.json"
        );
        let exchange: NativeActionStartAckExchange =
            serde_json::from_str(fixture).unwrap();
        let commit_count = Arc::new(AtomicUsize::new(0));
        let port = Arc::new(CommitPort {
            exchange: exchange.clone(),
            commit_count: Arc::clone(&commit_count),
        });
        let cancellation = CancellationToken::new();
        let serving = tokio::spawn({
            let client = Arc::clone(&client);
            let cancellation = cancellation.clone();
            async move { client.serve_ingress(port, cancellation).await }
        });

        let mut reader = BufReader::new(&mut server_reader);
        for id in [9, 10] {
            let request = serde_json::json!({
                "id": id,
                "method": "native_action/start",
                "params": &exchange.start,
            });
            server_writer
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
            server_writer.flush().await.unwrap();

            let mut response = String::new();
            reader.read_line(&mut response).await.unwrap();
            assert_eq!(commit_count.load(Ordering::Acquire), 1);
            let response: Value = serde_json::from_str(&response).unwrap();
            assert_eq!(
                response["result"],
                serde_json::to_value(&exchange.ack).unwrap()
            );
        }

        cancellation.cancel();
        serving.await.unwrap().unwrap();
        client.close().await;
    }
}
