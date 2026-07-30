//! `deepgram.listen` — Deepgram pre-recorded speech-to-text (ported from
//! `nomifun-shell/src/stt_deepgram.rs`).
//!
//! `POST {base}/v1/listen` (base from the connection, `'/'`-trimmed; an
//! `is_full_url` base is used verbatim; an explicit `params.endpoint`
//! override wins over both, routed through
//! [`crate::call::ResolvedCall::dispatch_target`]) with the raw audio bytes
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
use crate::adapters::has_endpoint_override;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_raw};
use crate::types::{TaskOutcome, TaskRequest, TaskResult};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

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

        // An explicit `params.endpoint` override wins (resolved verbatim by
        // the single dispatch authority); otherwise the conventional
        // `/v1/listen` path. Query params ride via `.query()` either way.
        let url = if has_endpoint_override(&call.model_params) {
            call.dispatch_target().url
        } else {
            let base = call.connection.base_url.trim().trim_end_matches('/');
            if call.connection.is_full_url { base.to_string() } else { format!("{base}/v1/listen") }
        };

        // Query string: model always; language when given, else detect it;
        // punctuate/smart_format default on, overridable via extra booleans.
        let language = req.language.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let mut query: Vec<(&str, String)> = vec![("model", call.model.clone())];
        if let Some(lang) = language {
            query.push(("language", lang.to_string()));
        } else {
            query.push(("detect_language", "true".to_string()));
        }
        if extra_flag(&req.extra, "punctuate") {
            query.push(("punctuate", "true".to_string()));
        }
        if extra_flag(&req.extra, "smart_format") {
            query.push(("smart_format", "true".to_string()));
        }

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
        let body: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid deepgram JSON: {e}")))?;

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

/// An `extra` boolean flag defaulting to `true` when absent or non-boolean.
fn extra_flag(extra: &Value, key: &str) -> bool {
    extra.get(key).and_then(|v| v.as_bool()).unwrap_or(true)
}

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
    use crate::types::{AsrRequest, InputAsset};

    /// A deepgram [`ResolvedCall`] as the resolver produces it: platform
    /// `deepgram`, default connection rewritten to the `Token` scheme.
    fn deepgram_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-0000000000bb".into(),
            platform: "deepgram".into(),
            model: model.into(),
            task,
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                is_full_url: false,
                auth: AuthMaterial {
                    scheme: AuthScheme::TokenHeader,
                    credentials: json!({"api_keys": ["dg-key"]}),
                },
                extra: json!({}),
            },
            model_params: json!({}),
            request,
        }
    }

    fn asr(language: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: b"RIFFdata".to_vec(), mime: "audio/wav".into() },
            language: language.map(str::to_string),
            prompt: None,
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

        let call = deepgram_call(&server.uri(), "nova-2", asr(Some("en"), json!({})));
        let out = DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
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

        let call = deepgram_call(&server.uri(), "nova-2", asr(None, json!({})));
        let out = DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
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

        let call = deepgram_call(
            &server.uri(),
            "nova-2",
            asr(Some("en"), json!({"punctuate": false, "smart_format": false})),
        );
        DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let query = requests[0].url.query().unwrap_or("");
        assert!(!query.contains("punctuate"), "punctuate must be omitted, got {query}");
        assert!(!query.contains("smart_format"), "smart_format must be omitted, got {query}");
    }

    #[tokio::test]
    async fn listen_full_url_base_is_used_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/listen"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = deepgram_call(&format!("{}/custom/listen", server.uri()), "nova-2", asr(None, json!({})));
        call.connection.is_full_url = true;
        DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn listen_params_endpoint_override_wins() {
        // Whole-branch review Finding 1: params.endpoint (dispatch rule 1)
        // must win over the /v1/listen convention; the query string still
        // rides via .query() on the overridden URL.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/asr"))
            .and(query_param("model", "nova-2"))
            .and(query_param("detect_language", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = deepgram_call(&server.uri(), "nova-2", asr(None, json!({})));
        call.model_params = json!({"endpoint": "/custom/asr"});
        let out = DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
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

        let call = deepgram_call(&server.uri(), "nova-2", asr(None, json!({})));
        let err = DeepgramListenAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("invalid token"), "message: {}", err.message);
    }
}
