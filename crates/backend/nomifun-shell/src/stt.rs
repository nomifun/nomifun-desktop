//! Speech-to-text over the unified invoke layer.
//!
//! Since the P1 invoke redesign this module no longer speaks any provider
//! protocol itself: the route validates the stored preference against the
//! provider catalog into a [`CloudSttRoute`], and [`SttService::transcribe`]
//! hands the audio to [`ModelInvokeService`], whose platform routing table
//! (openai.audio_transcriptions / deepgram.listen / …, overridable per model
//! row) decides the actual protocol. The frontend's stored `provider` enum
//! guess is ignored for execution.

use std::sync::Arc;

use nomifun_api_types::{SpeechToTextProvider, SpeechToTextResult};
use nomifun_model_invoke::{
    AsrRequest, InputAsset, InvokeError, InvokeErrorKind, ModelInvokeService, ModelRef,
    TaskOutcome, TaskRequest, TaskResult,
};

use crate::error::SttError;

/// A validated cloud speech-recognition selection: the catalog coordinates
/// the invoke layer resolves (provider row + model name) plus the display
/// metadata the wire response echoes. Produced by the route layer after it
/// has checked the stored preference against the provider catalog.
pub struct CloudSttRoute {
    pub provider_id: String,
    pub model: String,
    /// The provider row's platform. Display-only: it feeds the wire
    /// `provider` enum (see [`wire_provider`]), never protocol selection.
    pub platform: String,
    /// Preferred transcription language from the stored config
    /// (trimmed, non-empty). Wins over the per-request hint.
    pub language: Option<String>,
}

/// Speech-to-text service: a thin shim routing `/api/stt` transcriptions
/// through the unified [`ModelInvokeService`].
pub struct SttService {
    /// `None` mirrors `ShellRouterState::provider_service`: unit tests without
    /// a catalog leave it unwired and transcription degrades to a config error.
    invoke: Option<Arc<ModelInvokeService>>,
}

impl SttService {
    pub fn new(invoke: Option<Arc<ModelInvokeService>>) -> Self {
        Self { invoke }
    }

    pub async fn transcribe(
        &self,
        audio_data: Vec<u8>,
        mime_type: &str,
        language_hint: Option<&str>,
        route: &CloudSttRoute,
    ) -> Result<SpeechToTextResult, SttError> {
        let Some(invoke) = self.invoke.as_ref() else {
            return Err(SttError::Unknown(
                "model invoke service is unavailable for speech recognition".into(),
            ));
        };

        // Existing contract: the stored config language wins; the request's
        // languageHint only fills the gap.
        let language = route
            .language
            .clone()
            .or_else(|| language_hint.map(str::to_owned))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let model_ref =
            ModelRef { provider_id: route.provider_id.clone(), model: route.model.clone() };
        let request = TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".into(),
                bytes: audio_data,
                mime: mime_type.to_owned(),
            },
            language: language.clone(),
            prompt: None,
            extra: serde_json::json!({}),
        });

        let outcome = invoke.invoke(&model_ref, request).await.map_err(stt_error_from_invoke)?;
        let TaskOutcome::Done(TaskResult::Transcript {
            text,
            language: transcript_language,
            model,
        }) = outcome
        else {
            return Err(SttError::Unknown(
                "speech recognition returned an unexpected result shape".into(),
            ));
        };

        Ok(SpeechToTextResult {
            text,
            // Adapters echo the served model when the provider reports one
            // (deepgram's model_info); fall back to the selected model.
            model: model.unwrap_or_else(|| route.model.clone()),
            provider: wire_provider(&route.platform),
            language: transcript_language.or(language),
        })
    }
}

/// Map the resolved provider's platform onto the wire `provider` enum.
///
/// The field is display-only in the frontend; the execution protocol comes
/// from the invoke layer's platform routing (plus any model-row protocol
/// override). The honest mapping left: the deepgram platform reports
/// `deepgram`, everything else reports `openai`.
fn wire_provider(platform: &str) -> SpeechToTextProvider {
    if platform.eq_ignore_ascii_case("deepgram") {
        SpeechToTextProvider::Deepgram
    } else {
        SpeechToTextProvider::Openai
    }
}

/// Map an [`InvokeError`] onto the legacy [`SttError`] wire semantics: any
/// failure that reached (or failed reaching) the provider mirrors the old
/// `RequestFailed` (502); purely local resolution/config failures stay
/// `Unknown` (500), matching what the route's own validations return.
fn stt_error_from_invoke(error: InvokeError) -> SttError {
    use InvokeErrorKind as K;
    if error.http_status.is_some() {
        return SttError::RequestFailed(error.to_string());
    }
    match error.kind {
        K::Auth
        | K::RateLimited
        | K::QuotaExhausted
        | K::ContentPolicy
        | K::ProviderError
        | K::JobFailed
        | K::Network
        | K::Timeout
        | K::ParseError => SttError::RequestFailed(error.to_string()),
        K::UnsupportedTask
        | K::NoAdapter
        | K::MissingConnection
        | K::InvalidParams
        | K::NotPollable
        | K::Config => SttError::Unknown(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(platform: &str) -> CloudSttRoute {
        CloudSttRoute {
            provider_id: "018f0000-0000-7000-8000-000000000001".into(),
            model: "whisper-1".into(),
            platform: platform.into(),
            language: None,
        }
    }

    #[tokio::test]
    async fn unwired_invoke_service_is_unknown_error() {
        let svc = SttService::new(None);
        let result = svc.transcribe(vec![0u8; 4], "audio/wav", None, &route("openai")).await;
        assert!(matches!(result, Err(SttError::Unknown(msg)) if msg.contains("unavailable")));
    }

    #[test]
    fn wire_provider_maps_deepgram_platform_and_defaults_to_openai() {
        assert!(matches!(wire_provider("deepgram"), SpeechToTextProvider::Deepgram));
        assert!(matches!(wire_provider("openai"), SpeechToTextProvider::Openai));
        assert!(matches!(wire_provider("stepfun"), SpeechToTextProvider::Openai));
        assert!(matches!(wire_provider("custom"), SpeechToTextProvider::Openai));
    }

    #[test]
    fn invoke_errors_map_to_legacy_stt_semantics() {
        // Anything carrying an upstream HTTP status is a RequestFailed (502).
        let upstream = InvokeError {
            kind: InvokeErrorKind::InvalidParams,
            message: "provider returned 400 Bad Request: nope".into(),
            http_status: Some(400),
            retry_after_ms: None,
        };
        assert!(matches!(stt_error_from_invoke(upstream), SttError::RequestFailed(_)));

        // Transport failures never reached a status but are still upstream-ish.
        let network = InvokeError::new(InvokeErrorKind::Network, "request failed: connect");
        assert!(matches!(stt_error_from_invoke(network), SttError::RequestFailed(_)));

        // Local resolution/config failures stay Unknown (500).
        for kind in [
            InvokeErrorKind::UnsupportedTask,
            InvokeErrorKind::NoAdapter,
            InvokeErrorKind::MissingConnection,
            InvokeErrorKind::Config,
        ] {
            let error = InvokeError::new(kind, "local");
            assert!(
                matches!(stt_error_from_invoke(error), SttError::Unknown(_)),
                "kind {kind:?} must map to Unknown"
            );
        }
    }
}
