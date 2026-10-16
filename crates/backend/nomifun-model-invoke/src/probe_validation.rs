//! Protocol-independent checks on the exact normalized health-probe result.
//! A transport acknowledgement or a download URL is not a verified artifact.

use nomifun_api_types::HealthStatus;

use crate::adapters::audio_payload::validate_audio_payload;
use crate::error::InvokeError;
use crate::types::{ProducedData, TaskOutcome, TaskRequest, TaskResult};
use crate::ResolvedCall;

pub(super) fn validate_outcome(call: &ResolvedCall, outcome: &TaskOutcome) -> Result<HealthStatus, InvokeError> {
    match outcome {
        TaskOutcome::Pending(job) => {
            if job.adapter_id != call.protocol || job.remote_id.trim().is_empty() {
                return Err(InvokeError::parse("provider returned an invalid or foreign async job"));
            }
            Ok(HealthStatus::Unknown)
        }
        TaskOutcome::Done(result) => validate_result(&call.request, result),
    }
}

fn validate_result(request: &TaskRequest, result: &TaskResult) -> Result<HealthStatus, InvokeError> {
    match (request, result) {
        (TaskRequest::SpeechRecognition(_), TaskResult::Transcript { .. }) => {
            // Silence legitimately produces an empty transcript. Native codecs
            // must distinguish a present empty string from a missing field.
            Ok(HealthStatus::Healthy)
        }
        (TaskRequest::Embedding(request), TaskResult::Embeddings(vectors)) => {
            if vectors.len() != request.inputs.len() || vectors.is_empty() {
                return Err(InvokeError::parse("embedding probe returned an incorrect vector count"));
            }
            let dimensions = vectors[0].len();
            if dimensions == 0 || vectors.iter().any(|vector|
                vector.len() != dimensions || vector.iter().any(|value| !value.is_finite())) {
                return Err(InvokeError::parse("embedding probe returned empty, inconsistent or non-finite vectors"));
            }
            Ok(HealthStatus::Healthy)
        }
        (TaskRequest::Rerank(request), TaskResult::Reranked(results)) => {
            let maximum = request.top_n.map(|n| n as usize).unwrap_or(request.documents.len()).min(request.documents.len());
            let mut indices = std::collections::HashSet::new();
            if results.is_empty() || results.len() > maximum || results.iter().any(|result|
                result.index >= request.documents.len() || !indices.insert(result.index) || !result.relevance_score.is_finite()) {
                return Err(InvokeError::parse("rerank probe returned invalid scores, result counts or document indices"));
            }
            Ok(HealthStatus::Healthy)
        }
        (TaskRequest::ImageGeneration(_) | TaskRequest::ImageEdit(_) | TaskRequest::VideoGeneration(_)
            | TaskRequest::SpeechSynthesis(_) | TaskRequest::MusicGeneration(_), TaskResult::Assets(assets)) => {
            if assets.is_empty() {
                return Err(InvokeError::parse("media probe returned no assets"));
            }
            let maximum_images = match request {
                TaskRequest::ImageGeneration(request) => Some(request.count),
                TaskRequest::ImageEdit(request) => Some(request.count),
                _ => None,
            };
            if maximum_images.is_some_and(|count| assets.len() > count as usize) {
                return Err(InvokeError::parse("image probe returned more images than requested"));
            }
            let mut verified = true;
            for asset in assets {
                match &asset.data {
                    ProducedData::Url(raw) => {
                        crate::materialize::parse_artifact_url(raw)?;
                        verified = false; // Never fetch a provider URL merely to paint a green dot.
                    }
                    ProducedData::Bytes(bytes) => {
                        if bytes.is_empty() {
                            return Err(InvokeError::parse("media probe returned an empty artifact"));
                        }
                        match request {
                            TaskRequest::ImageGeneration(_) | TaskRequest::ImageEdit(_) => {
                                verified &= validate_image(bytes)?;
                            }
                            TaskRequest::SpeechSynthesis(_) | TaskRequest::MusicGeneration(_) => {
                                validate_audio_payload(bytes, asset.mime.as_deref(), "audio probe")?;
                            }
                            TaskRequest::VideoGeneration(_) => {
                                // Submission alone cannot establish a playable video.
                                if nomifun_net::api_response::looks_like_markup(bytes)
                                    || serde_json::from_slice::<serde::de::IgnoredAny>(bytes).is_ok() {
                                    return Err(InvokeError::parse("video probe returned a document instead of video"));
                                }
                                verified = false;
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            Ok(if verified { HealthStatus::Healthy } else { HealthStatus::Unknown })
        }
        _ => Err(InvokeError::parse("probe returned a result for a different task")),
    }
}

fn validate_image(bytes: &[u8]) -> Result<bool, InvokeError> {
    if nomifun_net::api_response::looks_like_markup(bytes)
        || serde_json::from_slice::<serde::de::IgnoredAny>(bytes).is_ok() {
        return Err(InvokeError::parse("image probe returned a document instead of an image"));
    }
    let format = image::guess_format(bytes).map_err(|_| InvokeError::parse("image probe returned unrecognized image bytes"))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP) {
        return Ok(false); // A known but unverified codec is not an upstream failure.
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| InvokeError::parse("image probe returned a damaged or oversized image"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use serde_json::json;

    fn image_request() -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest { prompt: "probe".into(), count: 1, size: None, quality: None, extra: json!({}) })
    }
    fn asset(data: ProducedData) -> TaskResult { TaskResult::Assets(vec![ProducedAsset { data, mime: None }]) }

    #[test]
    fn rejects_empty_invalid_and_wrong_task_outputs() {
        for result in [TaskResult::Assets(vec![]), asset(ProducedData::Bytes(vec![])),
            asset(ProducedData::Bytes(b"not an image".to_vec())),
            asset(ProducedData::Bytes(b"{\"error\":\"fake success\"}".to_vec())),
            TaskResult::Embeddings(vec![vec![1.0]])] {
            assert!(validate_result(&image_request(), &result).is_err());
        }
        for url in ["", " ", "file:///tmp/output", "https://user:secret@example.com/file", "https://example.com/file#fragment"] {
            assert!(validate_result(&image_request(), &asset(ProducedData::Url(url.into()))).is_err());
        }
    }

    #[test]
    fn only_decoded_images_are_healthy_and_remote_images_remain_unverified() {
        let png = super::super::probe_png();
        assert_eq!(validate_result(&image_request(), &asset(ProducedData::Bytes(png.clone()))).unwrap(), HealthStatus::Healthy);
        assert!(validate_result(&image_request(), &asset(ProducedData::Bytes(png[..24].to_vec()))).is_err());
        assert_eq!(validate_result(&image_request(), &asset(ProducedData::Url("https://example.com/output.png?signature=opaque".into()))).unwrap(), HealthStatus::Unknown);
    }

    #[test]
    fn speech_recognition_allows_a_present_empty_transcript() {
        let request = TaskRequest::SpeechRecognition(AsrRequest { audio: InputAsset { id: None, role: "audio".into(), bytes: vec![0, 0], mime: "audio/pcm".into() }, language: None, prompt: None, extra: json!({}) });
        assert_eq!(validate_result(&request, &TaskResult::Transcript { text: String::new(), language: None, model: None }).unwrap(), HealthStatus::Healthy);
    }

    #[test]
    fn retrieval_results_must_remain_finite_and_match_the_request() {
        let request = TaskRequest::Embedding(EmbedRequest { inputs: vec!["a".into(), "b".into()], extra: json!({}) });
        for vectors in [vec![], vec![vec![1.0]], vec![vec![], vec![]], vec![vec![1.0], vec![1.0, 2.0]], vec![vec![1.0], vec![f32::INFINITY]]] {
            assert!(validate_result(&request, &TaskResult::Embeddings(vectors)).is_err());
        }
        assert_eq!(validate_result(&request, &TaskResult::Embeddings(vec![vec![0.0], vec![-1.0]])).unwrap(), HealthStatus::Healthy);
        let request = TaskRequest::Rerank(RerankRequest { query: "a".into(), documents: vec!["a".into(), "b".into()], top_n: Some(2), extra: json!({}) });
        let result = |index, score| RerankResult { index, relevance_score: score, document: None };
        for results in [vec![], vec![result(2, 1.0)], vec![result(0, 1.0), result(0, 0.0)], vec![result(0, f32::NAN)]] {
            assert!(validate_result(&request, &TaskResult::Reranked(results)).is_err());
        }
        assert_eq!(validate_result(&request, &TaskResult::Reranked(vec![result(1, -2.0)])).unwrap(), HealthStatus::Healthy);
    }
}
