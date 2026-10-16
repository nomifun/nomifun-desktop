//! Google native batch embeddings and unary Interactions media protocols.
//! Interactions stay stateless: typed inputs win, streaming/storage are off,
//! and a caller cannot smuggle a previous server-side interaction into a call.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    ImageResponseBudget, MAX_ARTIFACT_BYTES, decode_b64, encode_b64, error_from_response,
    inline_image_response_body_limit, post_json, read_json_capped, validate_image_request_count,
};
use crate::types::{InputAsset, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest};

use super::json_request_body;

const EMBEDDING_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MEDIA_TIMEOUT: Duration = Duration::from_secs(180);
const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(60);
const JSON_OVERHEAD_BYTES: u64 = 1024 * 1024;

fn invalid(message: impl Into<String>) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

async fn native_post(
    http: &reqwest::Client,
    call: &ResolvedCall,
    body: &Value,
    timeout: Duration,
    max_bytes: u64,
) -> Result<Value, InvokeError> {
    let response = post_json(http, &call.endpoint_url()?, timeout, &call.connection.auth, body).await?;
    if !response.status().is_success() {
        return Err(error_from_response(response).await);
    }
    read_json_capped(response, max_bytes, "Google native response").await
}

/// BatchEmbedContents returns vectors in the exact order of its requests.
pub struct GeminiEmbeddingsAdapter;

#[async_trait]
impl ProtocolAdapter for GeminiEmbeddingsAdapter {
    fn id(&self) -> &'static str { "gemini.embeddings" }
    fn supports(&self, task: ModelTask) -> bool { task == ModelTask::Embedding }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::Embedding(request) = &call.request else {
            return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "gemini.embeddings requires an embedding request"));
        };
        if request.inputs.is_empty() { return Err(invalid("embeddings requires at least one input")); }
        let options = json_request_body(&call.model_params, &request.extra, json!({}))?;
        let mut options = options.as_object().expect("JSON helper returns an object").clone();
        // Options belong inside each EmbedContentRequest, not the batch root.
        for key in ["requests", "model", "input", "content", "contents", "stream", "store", "previous_interaction_id",
            "max_tokens_field", "chain_rounds", "require_reasoning_content"] { options.remove(key); }
        let dimension = options.get("outputDimensionality").map(|value| {
            value.as_u64().and_then(|value| usize::try_from(value).ok()).filter(|value| *value > 0)
                .ok_or_else(|| invalid("outputDimensionality must be a positive integer"))
        }).transpose()?;
        let model = call.model.trim().strip_prefix("models/").unwrap_or(call.model.trim());
        if model.is_empty() { return Err(invalid("Gemini embedding model is required")); }
        let requests = request.inputs.iter().map(|input| {
            let mut body = options.clone();
            body.insert("model".into(), json!(format!("models/{model}")));
            body.insert("content".into(), json!({"parts":[{"text":input}]}));
            Value::Object(body)
        }).collect::<Vec<_>>();
        let value = native_post(http, call, &json!({"requests":requests}), EMBEDDING_TIMEOUT, EMBEDDING_RESPONSE_BYTES).await?;
        let vectors = parse_embeddings(&value, request.inputs.len(), dimension)?;
        Ok(TaskOutcome::Done(TaskResult::Embeddings(vectors)))
    }
}

fn parse_embeddings(value: &Value, expected: usize, dimension: Option<usize>) -> Result<Vec<Vec<f32>>, InvokeError> {
    let embeddings = value.get("embeddings").and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("Gemini response is missing embeddings"))?;
    if embeddings.len() != expected || embeddings.is_empty() {
        return Err(InvokeError::parse("Gemini embedding count does not match the input count"));
    }
    let mut vectors = Vec::with_capacity(expected);
    let mut expected_dimension = dimension;
    for embedding in embeddings {
        let values = embedding.get("values").and_then(Value::as_array).filter(|values| !values.is_empty())
            .ok_or_else(|| InvokeError::parse("Gemini embedding values must be a non-empty array"))?;
        let dimension = *expected_dimension.get_or_insert(values.len());
        if values.len() != dimension { return Err(InvokeError::parse("Gemini embedding dimensions are inconsistent")); }
        let mut vector = Vec::with_capacity(values.len());
        for value in values {
            let number = value.as_f64().filter(|number| number.is_finite())
                .ok_or_else(|| InvokeError::parse("Gemini embedding contains a non-finite or non-numeric value"))?;
            let number = number as f32;
            if !number.is_finite() { return Err(InvokeError::parse("Gemini embedding value exceeds the finite f32 range")); }
            vector.push(number);
        }
        vectors.push(vector);
    }
    Ok(vectors)
}

/// Validate mergeable native option containers before typed fields are overlaid.
fn interaction_body(call: &ResolvedCall, extra: &Value, typed: Value) -> Result<Value, InvokeError> {
    let mut options = json_request_body(&call.model_params, extra, json!({}))?;
    let object = options.as_object_mut().expect("JSON helper returns an object");
    if object.get("previous_interaction_id").is_some_and(|value| !value.is_null()) {
        return Err(invalid("Stateless Gemini media calls cannot use previous_interaction_id"));
    }
    if object.get("background").is_some_and(|value| !value.is_null() && value != &Value::Bool(false)) {
        return Err(invalid("Unary Gemini media calls cannot run in background mode"));
    }
    if object.get("agent").is_some_and(|value| !value.is_null()) {
        return Err(invalid("Gemini media calls require a model, not a server-side agent"));
    }
    for key in ["response_format", "generation_config"] {
        if object.get(key).is_some_and(|value| !value.is_null() && !value.is_object()) {
            return Err(invalid(format!("Gemini {key} must be a JSON object")));
        }
    }
    for key in ["previous_interaction_id", "agent", "max_tokens_field", "chain_rounds", "require_reasoning_content"] { object.remove(key); }
    json_request_body(&options, &Value::Null, typed)
}

fn completed_interaction(value: &Value) -> Result<&[Value], InvokeError> {
    if value.get("status").is_some_and(|status| status.as_str() != Some("completed")) {
        return Err(InvokeError::new(InvokeErrorKind::ProviderError, "Gemini interaction did not reach completed status"));
    }
    value.get("steps").and_then(Value::as_array).map(Vec::as_slice)
        .ok_or_else(|| InvokeError::parse("Gemini interaction is missing steps"))
}

fn output_parts<'a>(steps: &'a [Value], kind: &'a str) -> impl Iterator<Item = &'a Value> {
    steps.iter().filter(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
        .filter_map(|step| step.get("content").and_then(Value::as_array))
        .flatten().filter(move |part| part.get("type").and_then(Value::as_str) == Some(kind) && part.get("thought") != Some(&Value::Bool(true)))
}

fn media_data(part: &Value) -> Result<(&str, &str), InvokeError> {
    let data = part.get("data").and_then(Value::as_str).filter(|data| !data.trim().is_empty())
        .ok_or_else(|| InvokeError::parse("Gemini media output is missing non-empty base64 data"))?;
    let mime = part.get("mime_type").and_then(Value::as_str).filter(|mime| !mime.trim().is_empty())
        .ok_or_else(|| InvokeError::parse("Gemini media output is missing its MIME type"))?;
    Ok((data, mime))
}

pub struct GeminiInteractionsImagesAdapter;

#[async_trait]
impl ProtocolAdapter for GeminiInteractionsImagesAdapter {
    fn id(&self) -> &'static str { "gemini.interactions_images" }
    fn supports(&self, task: ModelTask) -> bool { matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit) }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let (prompt, count, size, quality, inputs, extra): (&str, u32, Option<&str>, Option<&str>, &[InputAsset], &Value) = match &call.request {
            TaskRequest::ImageGeneration(request) => (&request.prompt, request.count, request.size.as_deref(), request.quality.as_deref(), &[], &request.extra),
            TaskRequest::ImageEdit(request) => {
                if request.inputs.is_empty() { return Err(invalid("Gemini image editing requires an input image")); }
                (&request.prompt, request.count, request.size.as_deref(), request.quality.as_deref(), &request.inputs, &request.extra)
            }
            _ => return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "gemini.interactions_images requires an image request")),
        };
        let count = validate_image_request_count(count)?;
        if quality.is_some() { return Err(invalid("Gemini Interactions has no equivalent generic image quality setting")); }
        let mut format = json!({"type":"image"});
        if let Some(size) = size {
            let (tier, square) = match size.trim().to_ascii_uppercase().as_str() {
                "1K" => ("1K", false), "2K" => ("2K", false), "4K" => ("4K", false),
                "1024X1024" => ("1K", true), "2048X2048" => ("2K", true), "4096X4096" => ("4K", true),
                _ => return Err(invalid("Gemini image size must be 1K/2K/4K or 1024x1024/2048x2048/4096x4096")),
            };
            format["image_size"] = json!(tier);
            if square { format["aspect_ratio"] = json!("1:1"); }
        }
        let mut input = vec![json!({"type":"text","text":prompt})];
        for image in inputs {
            if image.role == "mask" { return Err(invalid("Gemini Interactions does not map a separate mask input")); }
            let mime = image.mime.split(';').next().unwrap_or_default().trim();
            if !mime.starts_with("image/") || image.bytes.is_empty() {
                return Err(invalid("Gemini image inputs require non-empty image bytes and an image MIME type"));
            }
            input.push(json!({"type":"image","mime_type":mime,"data":encode_b64(&image.bytes)}));
        }
        let body = interaction_body(call, extra, json!({
            "model":call.model,"input":input,"response_format":format,"stream":false,"store":false,
        }))?;
        let mut budget = ImageResponseBudget::new(count)?;
        let mut assets = Vec::with_capacity(count);
        for _ in 0..count {
            let response = native_post(http, call, &body, MEDIA_TIMEOUT, inline_image_response_body_limit(1)).await?;
            assets.extend(parse_image_output(&response, &mut budget)?);
        }
        Ok(TaskOutcome::Done(TaskResult::Assets(assets)))
    }
}

fn parse_image_output(value: &Value, budget: &mut ImageResponseBudget) -> Result<Vec<ProducedAsset>, InvokeError> {
    let parts = output_parts(completed_interaction(value)?, "image").collect::<Vec<_>>();
    if parts.len() != 1 { return Err(InvokeError::parse("Gemini unary image response must contain exactly one final image")); }
    budget.ensure_additional_count(parts.len(), "Gemini Interactions image output")?;
    let (data, mime) = media_data(parts[0])?;
    if !mime.trim().to_ascii_lowercase().starts_with("image/") {
        return Err(InvokeError::parse("Gemini image output has a non-image MIME type"));
    }
    let bytes = budget.decode_base64(data, "Gemini Interactions image data")?;
    if bytes.is_empty() { return Err(InvokeError::parse("Gemini image output decoded to an empty image")); }
    Ok(vec![ProducedAsset { data: ProducedData::Bytes(bytes), mime: Some(mime.to_owned()) }])
}

pub struct GeminiInteractionsTtsAdapter;

fn audio_mime(format: &str) -> Result<&'static str, InvokeError> {
    match format.trim().to_ascii_lowercase().as_str() {
        "wav" | "audio/wav" => Ok("audio/wav"),
        "pcm" | "l16" | "audio/l16" => Ok("audio/l16"),
        "mulaw" | "audio/mulaw" => Ok("audio/mulaw"),
        "alaw" | "audio/alaw" => Ok("audio/alaw"),
        _ => Err(invalid("Gemini TTS supports wav, pcm, mulaw and alaw; the requested format cannot be represented")),
    }
}

fn tts_body(call: &ResolvedCall, request: &TtsRequest) -> Result<(Value, String), InvokeError> {
    let mut body = interaction_body(call, &request.extra, json!({
        "model":call.model,"input":[{"type":"user_input","content":[{"type":"text","text":request.text}]}],
        "response_format":{"type":"audio"},"stream":false,"store":false,
    }))?;
    let configured_voice = body.as_object_mut().expect("object").remove("voice");
    let voice = request.voice.as_deref().map(str::to_owned).or_else(|| configured_voice.as_ref().and_then(Value::as_str).map(str::to_owned));
    if request.voice.is_none() && configured_voice.as_ref().is_some_and(|value| !value.is_string()) {
        return Err(invalid("Gemini TTS voice must be a string"));
    }
    if voice.as_ref().is_some_and(|voice| voice.trim().is_empty()) { return Err(invalid("Gemini TTS voice must not be empty")); }
    let object = body.as_object_mut().expect("object");
    let config = object.entry("generation_config").or_insert_with(|| json!({}));
    if config.is_null() { *config = json!({}); }
    let config = config.as_object_mut().expect("native option container was validated");
    if let Some(voice) = voice {
        let speech = config.entry("speech_config").or_insert_with(|| json!([{}]));
        if speech.is_null() { *speech = json!([{}]); }
        let speaker = speech.as_array_mut().filter(|speakers| speakers.len() == 1)
            .and_then(|speakers| speakers[0].as_object_mut())
            .ok_or_else(|| invalid("A single Gemini voice cannot override a multi-speaker or malformed speech_config"))?;
        speaker.insert("voice".into(), json!(voice));
    }
    else {
        let speech = config.entry("speech_config").or_insert_with(|| json!([{"voice":"Kore"}]));
        if speech.is_null() { *speech = json!([{"voice":"Kore"}]); }
        if !speech.is_array() && !speech.is_object() { return Err(invalid("Gemini speech_config must be an array or object")); }
    }
    if request.format.is_none() && body["response_format"].get("mime_type").is_some_and(|value| !value.is_string()) {
        return Err(invalid("Gemini audio mime_type must be a supported audio MIME string"));
    }
    let configured_mime = body["response_format"].get("mime_type").and_then(Value::as_str);
    let mime = audio_mime(request.format.as_deref().or(configured_mime).unwrap_or("wav"))?.to_owned();
    body["response_format"]["mime_type"] = json!(mime);
    Ok((body, mime))
}

#[async_trait]
impl ProtocolAdapter for GeminiInteractionsTtsAdapter {
    fn id(&self) -> &'static str { "gemini.interactions_tts" }
    fn supports(&self, task: ModelTask) -> bool { task == ModelTask::SpeechSynthesis }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(request) = &call.request else {
            return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "gemini.interactions_tts requires a speech synthesis request"));
        };
        let (body, mime) = tts_body(call, request)?;
        let cap = MAX_ARTIFACT_BYTES.div_ceil(3) * 4 + JSON_OVERHEAD_BYTES;
        let value = native_post(http, call, &body, MEDIA_TIMEOUT, cap).await?;
        let assets = parse_audio_output(&value, &mime, MAX_ARTIFACT_BYTES as usize)?;
        Ok(TaskOutcome::Done(TaskResult::Assets(assets)))
    }
}

fn parse_audio_output(value: &Value, expected_mime: &str, limit: usize) -> Result<Vec<ProducedAsset>, InvokeError> {
    let mut remaining = limit;
    let mut assets = Vec::new();
    for part in output_parts(completed_interaction(value)?, "audio") {
        let (data, mime) = media_data(part)?;
        let actual = mime.split(';').next().unwrap_or_default().trim();
        if audio_mime(actual).ok() != Some(expected_mime) {
            return Err(InvokeError::parse("Gemini audio output MIME does not match the requested encoding"));
        }
        let data = data.trim();
        if data.len() > remaining.div_ceil(3).saturating_mul(4) {
            return Err(InvokeError::new(InvokeErrorKind::ProviderError, "Gemini audio exceeds the aggregate encoded budget"));
        }
        let bytes = decode_b64(data).filter(|bytes| !bytes.is_empty())
            .ok_or_else(|| InvokeError::parse("Gemini audio data must be non-empty valid base64"))?;
        remaining = remaining.checked_sub(bytes.len()).ok_or_else(|| InvokeError::new(
            InvokeErrorKind::ProviderError, "Gemini audio exceeds the aggregate decoded budget"))?;
        assets.push(ProducedAsset { data: ProducedData::Bytes(bytes), mime: Some(mime.to_owned()) });
    }
    if assets.is_empty() { return Err(InvokeError::parse("Gemini interaction returned no final audio")); }
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::auth::AuthScheme;
    use crate::types::{EmbedRequest, ImageEditRequest, ImageGenRequest};

    fn call(server: &str, protocol: &str, request: TaskRequest) -> ResolvedCall {
        let (model, endpoint) = if protocol == "gemini.embeddings" {
            ("gemini-embedding-001", "/v1beta/models/{model}:batchEmbedContents")
        } else if protocol == "gemini.interactions_images" {
            ("gemini-nano-banana-2.1", "/v1beta/interactions")
        } else { ("gemini-3.8-flash-tts", "/v1beta/interactions") };
        let mut call = call_with_endpoint(server, model, protocol, endpoint, request);
        call.platform = "gemini".into();
        call.connection.auth.scheme = AuthScheme::HeaderKey("x-goog-api-key".into());
        call.connection.auth.credentials = json!({"api_keys":["google-test-key"]});
        call
    }

    fn embed(inputs: &[&str]) -> TaskRequest {
        TaskRequest::Embedding(EmbedRequest { inputs: inputs.iter().map(|value| value.to_string()).collect(), extra: json!({}) })
    }
    fn image(count: u32, size: Option<&str>) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "  保留\n原始提示词  ".into(), count, size: size.map(str::to_owned), quality: None, extra: json!({}),
        })
    }
    fn speech(voice: Option<&str>, format: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest { text: "  Hello\n世界!  ".into(), voice: voice.map(str::to_owned), format: format.map(str::to_owned), extra: json!({}) })
    }
    fn interaction(kind: &str, mime: &str, bytes: &[u8]) -> Value {
        json!({"status":"completed","steps":[
            {"type":"thought","content":[{"type":kind,"mime_type":mime,"data":"invalid thought data"}]},
            {"type":"model_output","content":[{"type":kind,"mime_type":mime,"data":encode_b64(bytes)}]},
        ]})
    }
    fn assets(outcome: TaskOutcome) -> Vec<ProducedAsset> {
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected media assets") };
        assets
    }
    fn bytes(asset: &ProducedAsset) -> &[u8] {
        let ProducedData::Bytes(bytes) = &asset.data else { panic!("expected inline bytes") };
        bytes
    }

    #[tokio::test]
    async fn batch_embeddings_send_per_input_native_options_and_preserve_vector_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1beta/models/gemini-embedding-001:batchEmbedContents"))
            .and(header("x-goog-api-key", "google-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"embeddings":[{"values":[1.0,2.0]},{"values":[3.0,4.0]}]})))
            .expect(1).mount(&server).await;
        let mut call = call(&server.uri(), "gemini.embeddings", embed(&["  first\n", "第二项"]));
        call.model_params.as_object_mut().unwrap().extend(json!({
            "taskType":"RETRIEVAL_DOCUMENT","outputDimensionality":2,"title":"title",
            "model":"wrong-model","content":{"parts":[{"text":"wrong"}]},"requests":["wrong"],
            "credentials":{"api_keys":["must-not-leak"]},"vendor_option":{"enabled":true},
        }).as_object().unwrap().clone());
        let TaskOutcome::Done(TaskResult::Embeddings(vectors)) = GeminiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap()
            else { panic!("expected embeddings") };
        assert_eq!(vectors, vec![vec![1.0,2.0],vec![3.0,4.0]]);
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 1);
        for (index, input) in ["  first\n", "第二项"].into_iter().enumerate() {
            assert_eq!(body["requests"][index], json!({"model":"models/gemini-embedding-001","content":{"parts":[{"text":input}]},
                "taskType":"RETRIEVAL_DOCUMENT","outputDimensionality":2,"title":"title","vendor_option":{"enabled":true}}));
        }
    }

    #[test]
    fn embedding_response_rejects_missing_empty_mismatched_and_non_finite_vectors() {
        for (value, count, dimension) in [
            (json!({}), 1, None), (json!({"embeddings":[]}), 1, None),
            (json!({"embeddings":[{"values":[1]}]}), 2, None),
            (json!({"embeddings":[{"values":[]}]}), 1, None),
            (json!({"embeddings":[{"values":["1"]}]}), 1, None),
            (json!({"embeddings":[{"values":[null]}]}), 1, None),
            (json!({"embeddings":[{"values":[1e100]}]}), 1, None),
            (json!({"embeddings":[{"values":[1]},{"values":[1,2]}]}), 2, None),
            (json!({"embeddings":[{"values":[1]}]}), 1, Some(2)),
        ] { assert_eq!(parse_embeddings(&value, count, dimension).unwrap_err().kind, InvokeErrorKind::ParseError); }
    }

    #[tokio::test]
    async fn embedding_body_is_bounded_before_json_parsing() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
            .set_body_raw(vec![b' '; EMBEDDING_RESPONSE_BYTES as usize + 1], "application/json"))
            .expect(1).mount(&server).await;
        let error = GeminiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), "gemini.embeddings", embed(&["one"]))).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("cap"));
    }

    #[tokio::test]
    async fn image_count_loops_stateless_requests_and_ignores_thought_images() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1beta/interactions")).and(header("x-goog-api-key", "google-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(interaction("image", "image/png", b"image")))
            .expect(2).mount(&server).await;
        let mut call = call(&server.uri(), "gemini.interactions_images", image(2, Some("2048x2048")));
        call.model_params.as_object_mut().unwrap().extend(json!({"model":"injected","input":"injected","stream":true,"store":true,
            "response_format":{"type":"audio","aspect_ratio":"16:9","image_size":"4K"},
            "generation_config":{"temperature":0.4},"vendor_extra":{"flag":true},"headers":{"secret":"must-not-leak"}}).as_object().unwrap().clone());
        let result = assets(GeminiInteractionsImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap());
        assert_eq!(result.len(), 2); assert_eq!(bytes(&result[0]), b"image");
        let requests = server.received_requests().await.unwrap();
        for request in requests {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["input"], json!([{"type":"text","text":"  保留\n原始提示词  "}]));
            assert_eq!(body["model"], "gemini-nano-banana-2.1");
            assert_eq!(body["response_format"], json!({"type":"image","aspect_ratio":"1:1","image_size":"2K"}));
            assert_eq!(body["stream"], false); assert_eq!(body["store"], false);
            assert_eq!(body["generation_config"]["temperature"], 0.4);
            assert_eq!(body["vendor_extra"]["flag"], true);
            assert!(body.get("headers").is_none()); assert!(body.get("endpoint").is_none());
        }
    }

    #[tokio::test]
    async fn image_edit_sends_ordered_native_inline_image_blocks() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1beta/interactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(interaction("image", "image/jpeg", b"result"))).expect(1).mount(&server).await;
        let request = TaskRequest::ImageEdit(ImageEditRequest { prompt:"edit both".into(),count:1,size:Some("2K".into()),quality:None,
            inputs: vec![InputAsset{id:None,role:"image".into(),bytes:b"one".to_vec(),mime:"image/png".into()},
                InputAsset{id:None,role:"reference".into(),bytes:b"two".to_vec(),mime:"image/jpeg".into()}],extra:json!({"response_format":{"aspect_ratio":"3:2"}}) });
        let call = call(&server.uri(), "gemini.interactions_images", request);
        let result = assets(GeminiInteractionsImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap());
        assert_eq!(result[0].mime.as_deref(), Some("image/jpeg"));
        let received = server.received_requests().await.unwrap(); let body: Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(body["input"],json!([{"type":"text","text":"edit both"},{"type":"image","data":encode_b64(b"one"),"mime_type":"image/png"},
            {"type":"image","data":encode_b64(b"two"),"mime_type":"image/jpeg"}]));
        assert_eq!(body["response_format"],json!({"type":"image","image_size":"2K","aspect_ratio":"3:2"}));
    }

    #[tokio::test]
    async fn invalid_typed_image_controls_and_stateless_conflicts_fail_before_http() {
        let server = MockServer::start().await;
        let mut invalid_calls = vec![
            call(&server.uri(), "gemini.interactions_images", image(0,None)),
            call(&server.uri(), "gemini.interactions_images", image(9,None)),
            call(&server.uri(), "gemini.interactions_images", image(1,Some("512x512"))),
            call(&server.uri(), "gemini.interactions_images", image(1,Some("1024x1536"))),
        ];
        let mut quality = call(&server.uri(), "gemini.interactions_images", image(1,None));
        if let TaskRequest::ImageGeneration(request) = &mut quality.request { request.quality=Some("high".into()); }
        invalid_calls.push(quality);
        for options in [json!({"previous_interaction_id":"server-history"}),json!({"background":true}),json!({"agent":"server-agent"}),json!({"response_format":[]})] {
            let mut current=call(&server.uri(), "gemini.interactions_images", image(1,None));
            current.model_params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone()); invalid_calls.push(current);
        }
        for current in invalid_calls {
            assert_eq!(GeminiInteractionsImagesAdapter.submit(&reqwest::Client::new(), &current).await.unwrap_err().kind, InvokeErrorKind::InvalidParams);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn image_output_rejects_bad_data_mime_count_and_terminal_status() {
        let correct = interaction("image", "image/png", b"result");
        let mut duplicates = correct.clone();
        duplicates["steps"][1]["content"]=json!([{"type":"image","mime_type":"image/png","data":"aQ=="},{"type":"image","mime_type":"image/png","data":"aQ=="}]);
        let mut failed=correct.clone(); failed["status"]=json!("incomplete");
        let mut invalid=correct.clone(); invalid["steps"][1]["content"][0]["data"]=json!("not-base64");
        for value in [duplicates,failed,invalid,interaction("image","audio/wav",b"wrong"),interaction("image","image/png",b""),json!({"steps":[]})] {
            assert!(parse_image_output(&value,&mut ImageResponseBudget::new(1).unwrap()).is_err());
        }
        let mut budget=ImageResponseBudget::new(1).unwrap();
        parse_image_output(&correct,&mut budget).unwrap();
        assert!(parse_image_output(&correct,&mut budget).is_err());
    }

    #[tokio::test]
    async fn tts_maps_typed_voice_and_format_preserving_text_and_native_siblings() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1beta/interactions")).and(header("x-goog-api-key", "google-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(interaction("audio","audio/wav",b"RIFF-complete-WAV")))
            .expect(1).mount(&server).await;
        let mut call=call(&server.uri(),"gemini.interactions_tts",speech(Some("Zephyr"),Some("wav")));
        call.model_params.as_object_mut().unwrap().extend(json!({"voice":"Puck","stream":true,"store":true,
            "generation_config":{"temperature":0.7,"speech_config":[{"voice":"Aoede","language":"en-US"}]},
            "response_format":{"type":"image","sample_rate":16000},"credentials":{"secret":"hidden"}}).as_object().unwrap().clone());
        let result=assets(GeminiInteractionsTtsAdapter.submit(&reqwest::Client::new(),&call).await.unwrap());
        assert_eq!(bytes(&result[0]),b"RIFF-complete-WAV"); assert_eq!(result[0].mime.as_deref(),Some("audio/wav"));
        let requests=server.received_requests().await.unwrap();let body:Value=serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["input"],json!([{"type":"user_input","content":[{"type":"text","text":"  Hello\n世界!  "}]}]));
        assert_eq!(body["generation_config"],json!({"temperature":0.7,"speech_config":[{"voice":"Zephyr","language":"en-US"}]}));
        assert_eq!(body["response_format"],json!({"type":"audio","mime_type":"audio/wav","sample_rate":16000}));
        assert_eq!(body["stream"],false);assert_eq!(body["store"],false);assert!(body.get("voice").is_none());assert!(body.get("credentials").is_none());
    }

    #[test]
    fn tts_defaults_and_native_voice_configuration_are_not_replaced_by_openai_defaults() {
        let mut call=call("https://example.invalid","gemini.interactions_tts",speech(None,None));
        let TaskRequest::SpeechSynthesis(request)=&call.request else {unreachable!()};
        let (body,mime)=tts_body(&call,request).unwrap();assert_eq!(mime,"audio/wav");
        assert_eq!(body["generation_config"]["speech_config"],json!([{"voice":"Kore"}]));
        call.model_params["voice"]=json!("Puck");
        let (body,_)=tts_body(&call,request).unwrap();assert_eq!(body["generation_config"]["speech_config"],json!([{"voice":"Puck"}]));
        call.model_params.as_object_mut().unwrap().remove("voice");
        call.model_params["generation_config"]=json!({"speech_config":{"speakers":[{"speaker":"A","voice":"Aoede"}]}});
        let (body,_)=tts_body(&call,request).unwrap();
        assert_eq!(body["generation_config"],call.model_params["generation_config"]);
        assert!(body.get("previous_interaction_id").is_none());
    }

    #[tokio::test]
    async fn all_supported_tts_encodings_roundtrip_with_their_actual_mime_types() {
        for (format,mime) in [("wav","audio/wav"),("pcm","audio/l16"),("mulaw","audio/mulaw"),("alaw","audio/alaw")] {
            let server=MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(interaction("audio",mime,b"audio"))).expect(1).mount(&server).await;
            let call=call(&server.uri(),"gemini.interactions_tts",speech(None,Some(format)));
            let result=assets(GeminiInteractionsTtsAdapter.submit(&reqwest::Client::new(),&call).await.unwrap());
            assert_eq!(result[0].mime.as_deref(),Some(mime));
            let requests=server.received_requests().await.unwrap();let body:Value=serde_json::from_slice(&requests[0].body).unwrap();assert_eq!(body["response_format"]["mime_type"],mime);
        }
    }

    #[tokio::test]
    async fn unsupported_tts_formats_are_rejected_without_network_calls() {
        let server=MockServer::start().await;
        for format in ["mp3","ogg","flac","aac"] {
            let call=call(&server.uri(),"gemini.interactions_tts",speech(None,Some(format)));
            assert_eq!(GeminiInteractionsTtsAdapter.submit(&reqwest::Client::new(),&call).await.unwrap_err().kind,InvokeErrorKind::InvalidParams);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn audio_output_checks_encoded_and_aggregate_decoded_budgets_and_never_relabels_bytes() {
        assert!(parse_audio_output(&interaction("audio","audio/l16",b"audio"),"audio/wav",20).is_err());
        assert!(parse_audio_output(&interaction("audio","audio/mp3",b"audio"),"audio/wav",20).is_err());
        assert!(parse_audio_output(&interaction("audio","audio/wav",b"audio"),"audio/wav",3).is_err());
        let mut pair=interaction("audio","audio/wav",b"abcd");
        pair["steps"][1]["content"].as_array_mut().unwrap().push(json!({"type":"audio","mime_type":"audio/wav","data":encode_b64(b"efgh")}));
        assert!(parse_audio_output(&pair,"audio/wav",7).is_err());
        assert_eq!(parse_audio_output(&pair,"audio/wav",8).unwrap().len(),2);
        for status in ["failed","incomplete","in_progress","requires_action"] {
            let mut value=interaction("audio","audio/wav",b"audio");value["status"]=json!(status);
            assert!(parse_audio_output(&value,"audio/wav",20).is_err());
        }
        let mut malformed=interaction("audio","audio/wav",b"audio");malformed["steps"][1]["content"][0]["data"]=json!("invalid-base64");
        assert!(parse_audio_output(&malformed,"audio/wav",20).is_err());
    }
}
