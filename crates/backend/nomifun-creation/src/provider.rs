//! The [`MediaProvider`] adapter trait + its submit/poll value types (contract
//! §6 `provider.rs`). Adapters live under `adapters/` (empty in M0).

use async_trait::async_trait;
use serde_json::Value;

use crate::types::{CreationError, CreationInput, MediaCapability};

/// Everything an adapter needs to run one task: the resolved provider/model,
/// the capability, the opaque parameter map, and the input asset references.
///
/// M2 will extend this with the decrypted endpoint/key (resolved by the service
/// from the provider row) — kept out of M0 so the skeleton has no crypto/HTTP
/// surface yet.
#[derive(Debug, Clone)]
pub struct SubmitRequest {
    pub provider_id: String,
    pub model: String,
    pub capability: MediaCapability,
    /// Opaque parameter snapshot (prompt/size/quality/count/…).
    pub params: Value,
    pub inputs: Vec<CreationInput>,
}

/// A generated artifact handed back by an adapter: either inline bytes or a
/// URL the engine will fetch.
#[derive(Debug, Clone)]
pub struct ProducedAsset {
    pub data: ProducedData,
    /// MIME of the artifact when the adapter knows it.
    pub mime: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ProducedData {
    Bytes(Vec<u8>),
    Url(String),
}

/// The outcome of [`MediaProvider::submit`]: a synchronous protocol returns the
/// artifacts directly (`Done`); an async submit→poll protocol returns a remote
/// task id (`Pending`) the engine polls.
#[derive(Debug, Clone)]
pub enum SubmitAck {
    Done(Vec<ProducedAsset>),
    Pending { remote_task_id: String },
}

/// The outcome of one [`MediaProvider::poll`] tick.
#[derive(Debug, Clone)]
pub enum PollResult {
    Pending,
    Done(Vec<ProducedAsset>),
    Failed(CreationError),
}

/// A media generation backend. Adapter ids: `openai_images | media_async |
/// gemini_image | ark | modelscope | comfyui`.
#[async_trait]
pub trait MediaProvider: Send + Sync {
    /// Stable adapter id (matches the media-protocol tag on the provider).
    fn id(&self) -> &'static str;

    /// Whether this adapter can serve `cap`.
    fn supports(&self, cap: MediaCapability) -> bool;

    /// Kick off the job. `Done` for synchronous protocols; `Pending` for async
    /// submit→poll.
    async fn submit(&self, req: &SubmitRequest) -> Result<SubmitAck, CreationError>;

    /// Poll an async job by its remote id. `req` is the original request (for
    /// re-auth / endpoint reconstruction).
    async fn poll(&self, remote_task_id: &str, req: &SubmitRequest) -> Result<PollResult, CreationError>;
}
