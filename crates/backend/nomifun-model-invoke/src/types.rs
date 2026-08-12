//! Task request/response value types of the invocation layer.
//!
//! One request struct per [`nomifun_api_types::ModelTask`], folded into the
//! [`TaskRequest`] sum type the service dispatches on. Results normalize to
//! [`TaskResult`]; async providers return a [`JobHandle`] via
//! [`TaskOutcome::Pending`].

use serde::{Deserialize, Serialize};

/// Identifies the model to invoke: a provider row + a model name on it.
#[derive(Debug, Clone)]
pub struct ModelRef {
    pub provider_id: String,
    pub model: String,
}

/// A caller-supplied binary input (image / mask / audio…). Deliberately not
/// `Debug`: `bytes` may be large and must never end up in logs.
#[derive(Clone)]
pub struct InputAsset {
    pub id: Option<String>,
    /// Semantic slot of the asset within the request (e.g. `"image"`, `"mask"`).
    pub role: String,
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Text → image.
#[derive(Clone)]
pub struct ImageGenRequest {
    pub prompt: String,
    pub count: u32,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub extra: serde_json::Value,
}

/// Image(+mask)+text → image. The mask is the input with `role == "mask"`.
#[derive(Clone)]
pub struct ImageEditRequest {
    pub prompt: String,
    pub count: u32,
    pub size: Option<String>,
    pub inputs: Vec<InputAsset>,
    pub extra: serde_json::Value,
}

/// Text/image → video.
#[derive(Clone)]
pub struct VideoGenRequest {
    pub prompt: String,
    pub seconds: Option<u32>,
    pub size: Option<String>,
    pub inputs: Vec<InputAsset>,
    pub extra: serde_json::Value,
}

/// Text → speech.
#[derive(Clone)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
    pub format: Option<String>,
    pub extra: serde_json::Value,
}

/// Speech → text.
#[derive(Clone)]
pub struct AsrRequest {
    pub audio: InputAsset,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub extra: serde_json::Value,
}

/// Text → vector(s).
#[derive(Clone)]
pub struct EmbedRequest {
    pub inputs: Vec<String>,
    pub extra: serde_json::Value,
}

/// Query + candidate documents → relevance-ranked results.
#[derive(Clone)]
pub struct RerankRequest {
    pub query: String,
    pub documents: Vec<String>,
    pub top_n: Option<u32>,
    pub extra: serde_json::Value,
}

/// One normalized rerank result.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankResult {
    pub index: usize,
    pub relevance_score: f32,
    pub document: Option<String>,
}

/// The task-shaped request union the invocation service dispatches on.
#[derive(Clone)]
pub enum TaskRequest {
    ImageGeneration(ImageGenRequest),
    ImageEdit(ImageEditRequest),
    VideoGeneration(VideoGenRequest),
    SpeechSynthesis(TtsRequest),
    SpeechRecognition(AsrRequest),
    Embedding(EmbedRequest),
    Rerank(RerankRequest),
}

impl TaskRequest {
    /// The non-agent [`nomifun_api_types::ModelTask`] this request corresponds
    /// to. Chat and realtime sessions use their dedicated agent/session
    /// executors and therefore cannot be represented here.
    pub fn task(&self) -> nomifun_api_types::ModelTask {
        use nomifun_api_types::ModelTask;
        match self {
            Self::ImageGeneration(_) => ModelTask::ImageGeneration,
            Self::ImageEdit(_) => ModelTask::ImageEdit,
            Self::VideoGeneration(_) => ModelTask::VideoGeneration,
            Self::SpeechSynthesis(_) => ModelTask::SpeechSynthesis,
            Self::SpeechRecognition(_) => ModelTask::SpeechRecognition,
            Self::Embedding(_) => ModelTask::Embedding,
            Self::Rerank(_) => ModelTask::Rerank,
        }
    }
}

/// A produced artifact payload: inline bytes or a (short-lived) provider URL.
#[derive(Debug, Clone)]
pub enum ProducedData {
    Bytes(Vec<u8>),
    Url(String),
}

/// One produced artifact with its (optional) MIME type.
#[derive(Debug, Clone)]
pub struct ProducedAsset {
    pub data: ProducedData,
    pub mime: Option<String>,
}

/// Normalized result of a completed task.
#[derive(Debug, Clone)]
pub enum TaskResult {
    /// Media outputs (images / video / synthesized audio).
    Assets(Vec<ProducedAsset>),
    /// Speech-recognition transcript.
    Transcript { text: String, language: Option<String>, model: Option<String> },
    /// Embedding vectors, one per input.
    Embeddings(Vec<Vec<f32>>),
    /// Documents ranked by relevance to a query.
    Reranked(Vec<RerankResult>),
}

/// The normalized async-job handle (persisted as JSON between submit and poll).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobHandle {
    /// The [`crate::adapter::ProtocolAdapter::id`] that created the job.
    pub adapter_id: String,
    /// Monotonic revision of the complete provider invocation graph used at
    /// submit time. This field is required on the wire; resume fails closed if
    /// protocol or revision no longer matches the current capability.
    pub config_revision: i64,
    /// Provider-side (or client-generated) job identifier.
    pub remote_id: String,
    /// Adapter-private non-authority state (for example a provider request id
    /// echoed separately from `remote_id`). URLs, credentials and protocol
    /// configuration are always re-resolved from the current capability.
    #[serde(default)]
    pub poll_state: serde_json::Value,
}

/// Outcome of a submit/poll round: finished, or still pending with a handle.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Done(TaskResult),
    Pending(JobHandle),
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::ModelTask;
    use serde_json::json;

    use super::*;

    fn asset() -> InputAsset {
        InputAsset { id: None, role: "audio".into(), bytes: vec![1, 2, 3], mime: "audio/wav".into() }
    }

    #[test]
    fn task_request_maps_to_model_task() {
        let cases: Vec<(TaskRequest, ModelTask)> = vec![
            (
                TaskRequest::ImageGeneration(ImageGenRequest {
                    prompt: "p".into(),
                    count: 1,
                    size: None,
                    quality: None,
                    extra: json!({}),
                }),
                ModelTask::ImageGeneration,
            ),
            (
                TaskRequest::ImageEdit(ImageEditRequest {
                    prompt: "p".into(),
                    count: 1,
                    size: None,
                    inputs: vec![],
                    extra: json!({}),
                }),
                ModelTask::ImageEdit,
            ),
            (
                TaskRequest::VideoGeneration(VideoGenRequest {
                    prompt: "p".into(),
                    seconds: None,
                    size: None,
                    inputs: vec![],
                    extra: json!({}),
                }),
                ModelTask::VideoGeneration,
            ),
            (
                TaskRequest::SpeechSynthesis(TtsRequest {
                    text: "t".into(),
                    voice: None,
                    format: None,
                    extra: json!({}),
                }),
                ModelTask::SpeechSynthesis,
            ),
            (
                TaskRequest::SpeechRecognition(AsrRequest {
                    audio: asset(),
                    language: None,
                    prompt: None,
                    extra: json!({}),
                }),
                ModelTask::SpeechRecognition,
            ),
            (
                TaskRequest::Embedding(EmbedRequest { inputs: vec!["x".into()], extra: json!({}) }),
                ModelTask::Embedding,
            ),
            (
                TaskRequest::Rerank(RerankRequest {
                    query: "q".into(),
                    documents: vec!["x".into()],
                    top_n: Some(1),
                    extra: json!({}),
                }),
                ModelTask::Rerank,
            ),
        ];
        for (req, want) in cases {
            assert_eq!(req.task(), want);
        }
    }

    #[test]
    fn job_handle_roundtrips_and_defaults_poll_state() {
        let job = JobHandle { adapter_id: "ark.video_jobs".into(), config_revision: 7, remote_id: "j1".into(), poll_state: json!({"k": 1}) };
        let s = serde_json::to_string(&job).unwrap();
        let back: JobHandle = serde_json::from_str(&s).unwrap();
        assert_eq!(back.adapter_id, "ark.video_jobs");
        assert_eq!(back.config_revision, 7);
        assert_eq!(back.remote_id, "j1");
        assert_eq!(back.poll_state, json!({"k": 1}));

        // Adapter-private state is optional, but the invocation-graph revision
        // is deliberately not compatible with older naked handles.
        let bare: JobHandle = serde_json::from_str(
            r#"{"adapter_id":"a","config_revision":4,"remote_id":"r"}"#,
        )
        .unwrap();
        assert_eq!(bare.poll_state, serde_json::Value::Null);
        assert!(serde_json::from_str::<JobHandle>(r#"{"adapter_id":"a","remote_id":"r"}"#).is_err());
    }
}
