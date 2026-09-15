//! Rust Service SDK. stdout is reserved for the host protocol; log to stderr.
//! Native plugins are trusted OS processes, not a security sandbox. Build them
//! outside the host and import the resulting target-specific immutable release.
#![forbid(unsafe_code)]

pub use async_trait::async_trait;
use nomifun_agent_contracts::{
    NativePluginTarget, PluginServiceRuntimeFingerprint, PluginServiceStorageDescriptor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const VERSION: &str = nomifun_agent_contracts::PLUGIN_SERVICE_HOST_PROTOCOL_VERSION;
const MAX_FRAME: usize = 1024 * 1024;
const MAX_CALLS: usize = 128;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostContext {
    pub host_role: String,
    pub protocol_version: String,
    pub host_generation: u64,
    pub plugin_product_id: String,
    pub release: nomifun_agent_contracts::PluginReleaseRef,
    pub active_release_epoch: u64,
    pub service_run_key: nomifun_agent_contracts::DigestHex,
    pub module_path: String,
    pub module_digest: nomifun_agent_contracts::DigestHex,
    pub runtime: PluginServiceRuntimeFingerprint,
    pub storage: PluginServiceStorageDescriptor,
}

/// Implementations must yield during long work and must not detach per-call
/// tasks. Cancellation drops the invocation future; process-tree termination
/// remains the host's final enforcement boundary for uncooperative native code.
#[async_trait]
pub trait Service: Send + Sync + 'static {
    async fn start(&self, _host: &HostContext) -> Result<(), String> {
        Ok(())
    }
    async fn invoke(
        &self,
        method: &str,
        payload: Value,
        context: &mut CallContext,
    ) -> Result<Value, String>;
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}

type Writer = mpsc::Sender<Vec<u8>>;

pub struct CallContext {
    pub host: Arc<HostContext>,
    pub call_id: String,
    pub cancellation: CancellationToken,
    request_id: String,
    stream: bool,
    sequence: u64,
    storage_sequence: u64,
    writer: Writer,
    replies: mpsc::Receiver<Value>,
}

impl CallContext {
    /// Backpressure: returns only after the host accepts this event. Mutable
    /// borrowing disallows concurrent emit/storage operations on one call.
    pub async fn emit(&mut self, value: Value) -> Result<(), String> {
        if !self.stream {
            return Err("call did not request streaming".into());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("event sequence exhausted")?;
        self.write(json!({"kind":"event", "request_id":self.request_id,
            "call_id":self.call_id, "sequence":self.sequence, "value":value}))
            .await?;
        let reply = self.reply().await?;
        if reply["kind"] != "event_ack"
            || reply["request_id"] != self.request_id
            || reply["call_id"] != self.call_id
            || reply["sequence"] != self.sequence
        {
            return Err("event acknowledgement identity mismatch".into());
        }
        Ok(())
    }

    /// Uses the existing host-managed KV/database boundary; native code does
    /// not need (and does not receive) the host database connection.
    pub async fn storage(&mut self, operation: &str, payload: Value) -> Result<Value, String> {
        self.storage_sequence = self
            .storage_sequence
            .checked_add(1)
            .ok_or("storage sequence exhausted")?;
        let request_id = format!("{}:{}", self.request_id, self.storage_sequence);
        self.write(json!({"kind":"storage_request", "request_id":request_id,
            "parent_request_id":self.request_id, "operation":operation, "payload":payload}))
            .await?;
        let reply = self.reply().await?;
        if reply["kind"] != "storage_response" || reply["request_id"] != request_id {
            return Err("storage response identity mismatch".into());
        }
        if reply["outcome"] != "success" {
            return Err(reply["error"].to_string());
        }
        Ok(reply["value"].clone())
    }

    async fn write(&self, frame: Value) -> Result<(), String> {
        tokio::select! {
            _ = self.cancellation.cancelled() => Err("call canceled".into()),
            result = write_frame(&self.writer, self.host.host_generation, frame) => result.map_err(|e| e.to_string()),
        }
    }

    async fn reply(&mut self) -> Result<Value, String> {
        tokio::select! {
            _ = self.cancellation.cancelled() => Err("call canceled".into()),
            reply = self.replies.recv() => reply.ok_or("host disconnected".into()),
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

async fn write_frame(writer: &Writer, generation: u64, mut frame: Value) -> io::Result<()> {
    frame["protocol_version"] = VERSION.into();
    frame["host_generation"] = generation.into();
    let mut bytes = serde_json::to_vec(&frame)?;
    if bytes.len() > MAX_FRAME {
        return Err(invalid("frame exceeds 1 MiB"));
    }
    bytes.push(b'\n');
    writer
        .send(bytes)
        .await
        .map_err(|_| invalid("protocol writer stopped"))
}

struct ActiveCall {
    cancellation: CancellationToken,
    replies: mpsc::Sender<Value>,
}

/// Run a native executable under the production Service Host. The host passes
/// bootstrap metadata explicitly; no ambient Node installation is consulted.
pub async fn serve(service: impl Service) -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--nomifun-plugin-service" || args[1].len() > MAX_FRAME * 2 {
        return Err(invalid("expected --nomifun-plugin-service <bootstrap-hex>"));
    }
    let host: HostContext =
        serde_json::from_slice(&hex::decode(&args[1]).map_err(|e| invalid(e.to_string()))?)?;
    if host.host_role != "plugin_service"
        || host.protocol_version != VERSION
        || host.host_generation == 0
        || host.active_release_epoch == 0
        || !matches!(&host.runtime,
            PluginServiceRuntimeFingerprint::Native { native_target, native_executable_digest }
                if Some(*native_target) == NativePluginTarget::current() && *native_executable_digest == host.module_digest)
    {
        return Err(invalid("invalid native Service bootstrap"));
    }
    let host = Arc::new(host);
    let service = Arc::new(service);
    service.start(&host).await.map_err(invalid)?;
    let (writer, mut outgoing) = mpsc::channel::<Vec<u8>>(16);
    // One bounded writer owns stdout. Cancellation of an invocation must never
    // interrupt a half-written frame and corrupt all subsequent calls.
    let writing = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(bytes) = outgoing.recv().await {
            stdout.write_all(&bytes).await?;
            stdout.flush().await?;
        }
        io::Result::Ok(())
    });
    let result = run(host, service, writer).await;
    let flushed = writing.await.map_err(|e| invalid(e.to_string()))?;
    result.and(flushed)
}

async fn run(host: Arc<HostContext>, service: Arc<impl Service>, writer: Writer) -> io::Result<()> {
    write_frame(&writer, host.host_generation, json!({
        "kind":"hello", "host_role":host.host_role, "process_id":std::process::id(),
        "plugin_product_id":host.plugin_product_id, "release":host.release,
        "active_release_epoch":host.active_release_epoch, "service_run_key":host.service_run_key,
        "module_digest":host.module_digest, "runtime":host.runtime,
    })).await?;

    let mut reader = BufReader::new(tokio::io::stdin());
    let mut calls: BTreeMap<String, ActiveCall> = BTreeMap::new();
    let mut tasks = JoinSet::new();
    // Keep the read buffer across select cancellation: a partial NDJSON frame
    // must survive an invocation completing while stdin is being read.
    let mut line = Vec::new();
    loop {
        let mut bounded_reader = (&mut reader).take((MAX_FRAME + 1 - line.len()) as u64);
        tokio::select! {
            completed = tasks.join_next(), if !tasks.is_empty() => {
                let (request_id, call_id, result) = completed.expect("nonempty task set")
                    .map_err(|e| invalid(format!("Service handler failed: {e}")))?;
                calls.remove(&request_id);
                let response = match result {
                    Ok(value) => json!({"kind":"response", "request_id":request_id, "call_id":call_id, "outcome":"success", "value":value}),
                    Err(message) => json!({"kind":"response", "request_id":request_id, "call_id":call_id, "outcome":"failure",
                        "error":{"code":"service_invocation_failed", "message":message, "retryable":false}}),
                };
                write_frame(&writer, host.host_generation, response).await?;
            }
            count = bounded_reader.read_until(b'\n', &mut line) => {
                let count = count?;
                if count == 0 { return Err(invalid("host disconnected")); }
                if line.len() > MAX_FRAME || !line.ends_with(b"\n") { return Err(invalid("oversized or incomplete frame")); }
                let frame: Value = serde_json::from_slice(&line)?;
                line.clear();
                if frame["protocol_version"] != VERSION || frame["host_generation"] != host.host_generation {
                    return Err(invalid("host generation mismatch"));
                }
                let request_id = frame["request_id"].as_str().ok_or_else(|| invalid("missing request_id"))?.to_owned();
                match frame["kind"].as_str() {
                    Some("event_ack" | "storage_response") => {
                        let parent = if frame["kind"] == "storage_response" { request_id.split_once(':').map(|(parent, _)| parent).unwrap_or("") } else { &request_id };
                        if let Some(call) = calls.get(parent) {
                            // Replies to a just-canceled/finished invocation can
                            // cross its completion. A closed receiver is benign.
                            if let Err(mpsc::error::TrySendError::Full(_)) = call.replies.try_send(frame) { return Err(invalid("unsolicited host replies")); }
                        }
                    }
                    Some("control") if frame["operation"] == "cancel" => {
                        if let Some(call) = frame["target_request_id"].as_str().and_then(|id| calls.get(id)) { call.cancellation.cancel(); }
                        write_frame(&writer, host.host_generation, json!({"kind":"response", "request_id":request_id, "outcome":"ack"})).await?;
                    }
                    Some("control") if frame["operation"] == "shutdown" => {
                        for call in calls.values() { call.cancellation.cancel(); }
                        tasks.abort_all();
                        while tasks.join_next().await.is_some() {}
                        service.stop().await.map_err(invalid)?;
                        write_frame(&writer, host.host_generation, json!({"kind":"response", "request_id":request_id, "outcome":"ack"})).await?;
                        return Ok(());
                    }
                    Some("request") if frame["operation"] == "invoke" => {
                        if calls.len() >= MAX_CALLS || calls.contains_key(&request_id) || request_id.contains(':') { return Err(invalid("call capacity or identity violation")); }
                        let call_id = frame["call_id"].as_str().ok_or_else(|| invalid("missing call_id"))?.to_owned();
                        let method = frame["method"].as_str().ok_or_else(|| invalid("missing method"))?.to_owned();
                        let payload = frame["payload"].clone();
                        let cancellation = CancellationToken::new();
                        let (replies, receiver) = mpsc::channel(2);
                        calls.insert(request_id.clone(), ActiveCall { cancellation: cancellation.clone(), replies });
                        let mut context = CallContext { host: host.clone(), call_id: call_id.clone(), cancellation: cancellation.clone(),
                            request_id:request_id.clone(), stream:frame["stream"] == true, sequence:0, storage_sequence:0, writer:writer.clone(), replies:receiver };
                        let service = service.clone();
                        tasks.spawn(async move {
                            let result = tokio::select! {
                                biased;
                                _ = cancellation.cancelled() => Err("call canceled".into()),
                                result = service.invoke(&method, payload, &mut context) => result,
                            };
                            (request_id, call_id, result)
                        });
                    }
                    _ => return Err(invalid("unsupported Service frame")),
                }
            }
        }
    }
}
