//! Deepgram native speech protocols:
//! - `deepgram.listen` — pre-recorded speech-to-text (ported from
//!   `nomifun-shell/src/stt_deepgram.rs`).
//! - `deepgram.speak_rest` — synchronous REST text-to-speech. Deepgram's
//!   voice is the selected Aura model id, not a separate `voice` body field.
//!
//! The selected capability supplies the exact listen or speak endpoint,
//! resolved through [`crate::call::ResolvedCall::endpoint_url`]. Listen sends raw audio bytes
//! as the body
//! and `Content-Type` set to the audio MIME. Options ride the query string:
//! `model` always; `language` when the request carries one, else
//! `detect_language=true`; `punctuate=true` / `smart_format=true` default on
//! but can be disabled via `extra.punctuate` / `extra.smart_format` booleans.
//! Auth is applied declaratively ([`crate::auth::AuthMaterial::apply`] — the
//! resolver rewrites deepgram default connections to the `Token` scheme).
//!
//! The response parses to [`TaskResult::Transcript`]:
//! `results.channels[0].alternatives[0].transcript` as the text,
//! `metadata.model_info` (first value's `name`, falling back to the requested
//! model) as the model, and `results.channels[0].detected_language` (falling
//! back to the requested language) as the language.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::Value;

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    MAX_ARTIFACT_BYTES, error_from_response, post_json, post_raw, read_body_capped,
};
use crate::types::{
    ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest,
};

use super::scalar_request_fields;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DEEPGRAM_SPEAK_MAX_CHARS: usize = 2_000;

/// Deepgram pre-recorded `/v1/listen` protocol.
pub struct DeepgramListenAdapter;

#[async_trait]
impl ProtocolAdapter for DeepgramListenAdapter {
    fn id(&self) -> &'static str {
        "deepgram.listen"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("deepgram.listen cannot serve task {:?}", call.request.task()),
            ));
        };

        let url = call.endpoint_url()?;

        // Query string: model always; language when given, else detect it;
        // punctuate/smart_format default on, overridable via extra booleans.
        let language = req.language.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let mut query_fields = scalar_request_fields(&call.model_params, &req.extra)?;
        query_fields.insert("model".into(), call.model.clone());
        if let Some(lang) = language {
            query_fields.insert("language".into(), lang.to_string());
            query_fields.remove("detect_language");
        } else {
            query_fields.remove("language");
            query_fields.insert("detect_language".into(), "true".to_string());
        }
        query_fields.entry("punctuate".into()).or_insert_with(|| "true".into());
        query_fields.entry("smart_format".into()).or_insert_with(|| "true".into());
        let query_owned = query_fields.into_iter().collect::<Vec<_>>();
        let query = query_owned
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();

        let resp = post_raw(
            http,
            &url,
            REQUEST_TIMEOUT,
            &call.connection.auth,
            req.audio.mime.as_str(),
            &query,
            &req.audio.bytes,
        )
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid deepgram JSON", &e))?;

        let transcript = body["results"]["channels"]
            .get(0)
            .and_then(|ch| ch["alternatives"].get(0))
            .and_then(|alt| alt["transcript"].as_str())
            .unwrap_or("")
            .to_owned();
        let detected_language = body["results"]["channels"]
            .get(0)
            .and_then(|ch| ch["detected_language"].as_str())
            .map(str::to_owned)
            .or_else(|| language.map(str::to_owned));
        let model = extract_model_name(&body).unwrap_or_else(|| call.model.clone());

        Ok(TaskOutcome::Done(TaskResult::Transcript {
            text: transcript,
            language: detected_language,
            model: Some(model),
        }))
    }
}

/// Deepgram synchronous REST `/v1/speak` protocol.
pub struct DeepgramSpeakRestAdapter;

#[async_trait]
impl ProtocolAdapter for DeepgramSpeakRestAdapter {
    fn id(&self) -> &'static str {
        "deepgram.speak_rest"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!(
                    "deepgram.speak_rest cannot serve task {:?}",
                    call.request.task()
                ),
            ));
        };

        let text_chars = req.text.chars().count();
        if text_chars == 0 || text_chars > DEEPGRAM_SPEAK_MAX_CHARS {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "Deepgram speak text must contain 1-{DEEPGRAM_SPEAK_MAX_CHARS} characters (received {text_chars})"
                ),
            ));
        }

        let raw_url = call.endpoint_url()?;
        let url = build_speak_url(&raw_url, &call.model, req, &call.model_params)?;
        let body = serde_json::json!({ "text": req.text });

        let resp = post_json(
            http,
            url.as_str(),
            REQUEST_TIMEOUT,
            &call.connection.auth,
            &body,
        )
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase())
            .filter(|value| value.starts_with("audio/"))
            .unwrap_or_else(|| mime_for_speak_url(&url).to_owned());
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;

        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(mime),
        }])))
    }
}

fn build_speak_url(
    raw_url: &str,
    model: &str,
    req: &TtsRequest,
    model_params: &Value,
) -> Result<reqwest::Url, InvokeError> {
    let mut url = reqwest::Url::parse(raw_url)
        .map_err(|_| InvokeError::config("Deepgram speak endpoint is not a valid absolute URL"))?;
    for (key, value) in scalar_request_fields(model_params, &req.extra)? {
        replace_query_param(&mut url, &key, &value);
    }
    replace_query_param(&mut url, "model", model);

    let format = req.format.as_deref().map(str::trim).filter(|value| !value.is_empty());
    if let Some(format) = format {
        let (encoding, container) = match format.to_ascii_lowercase().as_str() {
            "mp3" => ("mp3", None),
            "wav" => ("linear16", Some("wav")),
            "opus" | "ogg" => ("opus", Some("ogg")),
            "flac" => ("flac", None),
            "aac" => ("aac", None),
            "pcm" => ("linear16", Some("none")),
            _ => {
                return Err(InvokeError::new(
                    InvokeErrorKind::InvalidParams,
                    format!("unsupported Deepgram speech format '{format}'"),
                ));
            }
        };
        replace_query_param(&mut url, "encoding", encoding);
        if let Some(container) = container {
            replace_query_param(&mut url, "container", container);
        } else {
            remove_query_param(&mut url, "container");
        }
    }

    Ok(url)
}

fn replace_query_param(url: &mut reqwest::Url, key: &str, value: &str) {
    let existing = url
        .query_pairs()
        .filter(|(name, _)| name != key)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    let mut query = url.query_pairs_mut();
    for (name, value) in existing {
        query.append_pair(&name, &value);
    }
    query.append_pair(key, value);
}

fn remove_query_param(url: &mut reqwest::Url, key: &str) {
    let existing = url
        .query_pairs()
        .filter(|(name, _)| name != key)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    let mut query = url.query_pairs_mut();
    for (name, value) in existing {
        query.append_pair(&name, &value);
    }
}

fn mime_for_speak_url(url: &reqwest::Url) -> &'static str {
    let mut encoding = None;
    let mut container = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "encoding" => encoding = Some(value.into_owned()),
            "container" => container = Some(value.into_owned()),
            _ => {}
        }
    }
    match (encoding.as_deref(), container.as_deref()) {
        (Some("mp3"), _) => "audio/mpeg",
        (Some("opus"), Some("ogg")) => "audio/ogg",
        (Some("opus"), _) => "audio/opus",
        (Some("flac"), _) => "audio/flac",
        (Some("aac"), _) => "audio/aac",
        (Some("linear16"), Some("none")) => "audio/l16",
        (Some("linear16" | "mulaw" | "alaw"), Some("wav")) => "audio/wav",
        (Some("mulaw"), Some("none")) => "audio/mulaw",
        (Some("alaw"), Some("none")) => "audio/alaw",
        // Deepgram's REST default encoding is MP3.
        _ => "audio/mpeg",
    }
}

/// An `extra` boolean flag defaulting to `true` when absent or non-boolean.
/// The model name Deepgram actually served with:
/// `metadata.model_info.<first key>.name`. Pure — unit tested (ported fixtures).
fn extract_model_name(body: &Value) -> Option<String> {
    body["metadata"]["model_info"]
        .as_object()
        .and_then(|map| map.values().next())
        .and_then(|info| info["name"].as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::ResolvedConnection;
    use crate::types::{AsrRequest, InputAsset, TtsRequest};

    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// A deepgram [`ResolvedCall`] as the resolver produces it: platform
    /// `deepgram`, default connection rewritten to the `Token` scheme.
    fn deepgram_call(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-0000000000bb".into(),
            config_revision: 1,
            platform: "deepgram".into(),
            model: model.into(),
            task,
            protocol: protocol.into(),
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                auth: AuthMaterial {
                    scheme: AuthScheme::TokenHeader,
                    credentials: json!({"api_keys": ["dg-key"]}),
                },
                extra: json!({}),
            },
            model_params: json!({"endpoint": endpoint}),
            request,
        }
    }

    fn listen_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        deepgram_call(base_url, model, "deepgram.listen", "/v1/listen", request)
    }

    fn speak_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        deepgram_call(base_url, model, "deepgram.speak_rest", "/v1/speak", request)
    }

    fn asr(language: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: b"RIFFdata".to_vec(), mime: "audio/wav".into() },
            language: language.map(str::to_string),
            prompt: None,
            extra,
        })
    }

    fn tts(text: &str, voice: Option<&str>, format: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.to_owned(),
            voice: voice.map(str::to_owned),
            format: format.map(str::to_owned),
            extra,
        })
    }

    fn transcript_body() -> Value {
        json!({
            "metadata": {
                "model_info": {
                    "some-uuid": { "name": "2-general-nova", "version": "2024-01-18.26916" }
                }
            },
            "results": {
                "channels": [{
                    "alternatives": [{ "transcript": "hello" }]
                }]
            }
        })
    }

    // -- ported pure-parser fixtures (stt_deepgram.rs) -----------------------

    #[test]
    fn extract_model_name_from_response() {
        assert_eq!(extract_model_name(&transcript_body()), Some("2-general-nova".to_owned()));
    }

    #[test]
    fn extract_model_name_missing_metadata() {
        let body = json!({
            "results": { "channels": [{ "alternatives": [{ "transcript": "hi" }] }] }
        });
        assert_eq!(extract_model_name(&body), None);
    }

    #[test]
    fn extract_model_name_empty_model_info() {
        let body = json!({
            "metadata": { "model_info": {} },
            "results": { "channels": [{ "alternatives": [{ "transcript": "hi" }] }] }
        });
        assert_eq!(extract_model_name(&body), None);
    }

    #[test]
    fn speak_mime_fallback_matches_deepgram_encoding_and_container() {
        for (query, expected) in [
            ("encoding=linear16&container=wav", "audio/wav"),
            ("encoding=linear16&container=none", "audio/l16"),
            ("encoding=mulaw&container=wav", "audio/wav"),
            ("encoding=mulaw&container=none", "audio/mulaw"),
            ("encoding=alaw&container=none", "audio/alaw"),
            ("encoding=opus&container=ogg", "audio/ogg"),
            ("encoding=mp3", "audio/mpeg"),
            ("", "audio/mpeg"),
        ] {
            let url = reqwest::Url::parse(&format!("https://api.deepgram.com/v1/speak?{query}"))
                .unwrap();
            assert_eq!(mime_for_speak_url(&url), expected, "query={query}");
        }
    }

    // -- wiremock request/response tests -------------------------------------

    #[tokio::test]
    async fn listen_sends_token_auth_query_params_and_raw_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/listen"))
            .and(header("authorization", "Token dg-key"))
            .and(header("content-type", "audio/wav"))
            .and(query_param("model", "nova-2"))
            .and(query_param("language", "en"))
            .and(query_param("punctuate", "true"))
            .and(query_param("smart_format", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
            .expect(1)
            .mount(&server)
            .await;

        let call = listen_call(&server.uri(), "nova-2", asr(Some("en"), json!({})));
        let out = DeepgramListenAdapter.submit(&no_proxy_client(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello");
        // No detected_language in the response → falls back to the request language.
        assert_eq!(language.as_deref(), Some("en"));
        assert_eq!(model.as_deref(), Some("2-general-nova"));

        // The raw audio bytes travel as the body, unencoded.
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].body, b"RIFFdata");
    }

    #[tokio::test]
    async fn listen_without_language_requests_detection_and_reads_detected() {
        let server = MockServer::start().await;
        let mut body = transcript_body();
        body["results"]["channels"][0]["detected_language"] = json!("es");
        Mock::given(method("POST"))
            .and(path("/v1/listen"))
            .and(query_param("detect_language", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let call = listen_call(&server.uri(), "nova-2", asr(None, json!({})));
        let out = DeepgramListenAdapter.submit(&no_proxy_client(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { language, .. }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(language.as_deref(), Some("es"));

        let requests = server.received_requests().await.unwrap();
        let query = requests[0].url.query().unwrap_or("");
        assert!(!query.contains("language=en"), "no language param expected, got {query}");
    }

    #[tokio::test]
    async fn listen_extra_booleans_disable_defaults() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/listen"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
            .expect(1)
            .mount(&server)
            .await;

        let call = listen_call(
            &server.uri(),
            "nova-2",
            asr(Some("en"), json!({"punctuate": false, "smart_format": false})),
        );
        DeepgramListenAdapter.submit(&no_proxy_client(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let query = requests[0].url.query().unwrap_or("");
        assert!(query.contains("punctuate=false"), "explicit false must be sent, got {query}");
        assert!(query.contains("smart_format=false"), "explicit false must be sent, got {query}");
    }

    #[tokio::test]
    async fn speak_uses_model_as_voice_and_returns_binary_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speak"))
            .and(header("authorization", "Token dg-key"))
            .and(query_param("model", "aura-2-thalia-en"))
            .and(query_param("encoding", "linear16"))
            .and(query_param("container", "wav"))
            .and(query_param("speed", "1.1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/wav")
                    .set_body_bytes(b"RIFFaudio".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = speak_call(
            &server.uri(),
            "aura-2-thalia-en",
            tts("hello", Some("must-not-be-sent"), Some("wav"), json!({"speed": 1.1})),
        );
        let out = DeepgramSpeakRestAdapter
            .submit(&no_proxy_client(), &call)
            .await
            .unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else {
            panic!("expected Done(Assets)")
        };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
        let ProducedData::Bytes(bytes) = &assets[0].data else {
            panic!("expected inline audio bytes")
        };
        assert_eq!(bytes, b"RIFFaudio");

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body, json!({"text": "hello"}));
        assert!(body.get("voice").is_none());
        assert!(body.get("model").is_none());
    }

    #[tokio::test]
    async fn speak_honors_endpoint_override_and_model_query_settings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            // Relative endpoint overrides are resolved against the complete
            // configured provider root, including its `/v1` prefix.
            .and(path("/v1/custom/speak"))
            .and(query_param("tag", "stable"))
            .and(query_param("model", "future-voice-id"))
            .and(query_param("encoding", "mp3"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ID3audio".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = deepgram_call(
            &format!("{}/v1", server.uri()),
            "future-voice-id",
            "deepgram.speak_rest",
            "/custom/speak?tag=stable",
            tts("hello", None, None, json!({})),
        );
        call.model_params = json!({
            "endpoint": "/custom/speak?tag=stable",
            "encoding": "mp3"
        });
        let out = DeepgramSpeakRestAdapter
            .submit(&no_proxy_client(), &call)
            .await
            .unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else {
            panic!("expected Done(Assets)")
        };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));
    }

    #[tokio::test]
    async fn speak_rejects_text_beyond_deepgram_limit_before_transport() {
        let call = speak_call(
            "https://api.deepgram.com",
            "aura-2-thalia-en",
            tts(&"x".repeat(DEEPGRAM_SPEAK_MAX_CHARS + 1), None, None, json!({})),
        );
        let error = DeepgramSpeakRestAdapter
            .submit(&no_proxy_client(), &call)
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("1-2000"));
    }

    #[tokio::test]
    async fn listen_params_endpoint_override_wins() {
        // The explicitly injected endpoint retains its query parameters.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/asr"))
            .and(query_param("model", "nova-2"))
            .and(query_param("detect_language", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
            .expect(1)
            .mount(&server)
            .await;

        let call = deepgram_call(
            &server.uri(),
            "nova-2",
            "deepgram.listen",
            "/custom/asr",
            asr(None, json!({})),
        );
        let out = DeepgramListenAdapter.submit(&no_proxy_client(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, .. }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/listen"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid token"))
            .mount(&server)
            .await;

        let call = listen_call(&server.uri(), "nova-2", asr(None, json!({})));
        let err = DeepgramListenAdapter.submit(&no_proxy_client(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("invalid token"), "message: {}", err.message);
    }
}
