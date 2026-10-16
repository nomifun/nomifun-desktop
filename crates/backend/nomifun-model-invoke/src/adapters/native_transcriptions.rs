//! Explicit synchronous multipart ASR transports. MiniMax's language hint is
//! an HTTP header; Zhipu supports prompt but not the OpenAI language field;
//! SiliconFlow documents file/model only. No OpenAI response fields are added
//! to the latter two protocols.
use std::time::Duration;
use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::header::HeaderValue;
use reqwest::multipart::{Form,Part};
use serde_json::Value;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError,InvokeErrorKind};
use crate::transport::{error_from_response,read_json_capped,response_secret_redactor,send_with_rotation};
use crate::types::{TaskOutcome,TaskRequest,TaskResult};
use super::scalar_request_fields;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TRANSCRIPTION_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone,Copy,PartialEq,Eq)]
enum NativeAsr {MiniMax,Zhipu,SiliconFlow}
impl NativeAsr {
    fn id(self) -> &'static str {match self {
        Self::MiniMax => "minimax.speech_to_text",Self::Zhipu => "zhipu.audio_transcriptions",Self::SiliconFlow => "siliconflow.audio_transcriptions",
    }}
}

pub struct MiniMaxSpeechToTextAdapter;
pub struct ZhipuAudioTranscriptionsAdapter;
pub struct SiliconFlowAudioTranscriptionsAdapter;

macro_rules! native_adapter {
    ($name:ident,$native:expr) => {
        #[async_trait]
        impl ProtocolAdapter for $name {
            fn id(&self) -> &'static str {$native.id()}
            fn supports(&self, task: ModelTask) -> bool {task == ModelTask::SpeechRecognition}
            async fn submit(&self,http:&reqwest::Client,call:&ResolvedCall) -> Result<TaskOutcome,InvokeError> {
                submit($native,http,call).await
            }
        }
    };
}
native_adapter!(MiniMaxSpeechToTextAdapter,NativeAsr::MiniMax);
native_adapter!(ZhipuAudioTranscriptionsAdapter,NativeAsr::Zhipu);
native_adapter!(SiliconFlowAudioTranscriptionsAdapter,NativeAsr::SiliconFlow);

fn invalid(native:NativeAsr,detail:&str) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams,format!("{} {detail}",native.id()))
}

async fn submit(native:NativeAsr,http:&reqwest::Client,call:&ResolvedCall) -> Result<TaskOutcome,InvokeError> {
    let TaskRequest::SpeechRecognition(req) = &call.request else {
        return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask,format!("{} requires speech_recognition",native.id())));
    };
    // Reject unsupported typed semantics before any request. Opaque native
    // scalar fields remain an explicit escape hatch, without guessed encodings.
    if native != NativeAsr::MiniMax && req.language.is_some() {
        return Err(invalid(native,"does not support the typed language parameter"));
    }
    if native != NativeAsr::Zhipu && req.prompt.is_some() {
        return Err(invalid(native,"does not support the typed prompt parameter"));
    }
    let url = call.endpoint_url()?;
    let mut fields = scalar_request_fields(&call.model_params,&req.extra)?;
    fields.remove("file");
    fields.insert("model".into(),call.model.clone());
    if let Some(stream) = fields.get("stream") {
        if stream != "false" {return Err(invalid(native,"supports synchronous stream=false only"));}
    }
    let language = if native == NativeAsr::MiniMax {
        let language = req.language.clone().or_else(|| fields.remove("language"));
        fields.remove("language");
        if let Some(format) = fields.get("response_format") {
            if !matches!(format.as_str(),"json"|"verbose_json") {
                return Err(invalid(native,"requires a JSON response_format (json or verbose_json)"));
            }
        } else {fields.insert("response_format".into(),"json".into());}
        language
    } else {
        if fields.contains_key("response_format") {return Err(invalid(native,"does not define response_format"));}
        None
    };
    if let Some(prompt) = &req.prompt {fields.insert("prompt".into(),prompt.clone());}
    let language_header = language.as_deref().map(str::trim).map(HeaderValue::from_str).transpose()
        .map_err(|_| invalid(native,"language must be a valid HTTP header value"))?;
    let make_form = || -> Result<Form,InvokeError> {
        let part = Part::bytes(req.audio.bytes.clone()).file_name(format!("audio.{}",audio_extension(&req.audio.mime)))
            .mime_str(&req.audio.mime).map_err(|_| invalid(native,"audio MIME is invalid"))?;
        let mut form = Form::new();
        for (key,value) in &fields {form = form.text(key.clone(),value.clone());}
        Ok(form.part("file",part))
    };
    let response = send_with_rotation(&call.connection.auth,|| {
        let mut request = http.post(&url).timeout(REQUEST_TIMEOUT).multipart(make_form()?);
        if let Some(language) = &language_header {request = request.header("language",language.clone());}
        Ok(request)
    }).await?;
    if !response.status().is_success() {return Err(error_from_response(response).await);}
    let redactor = response_secret_redactor(&response);
    let body:Value = read_json_capped(response,MAX_TRANSCRIPTION_RESPONSE_BYTES,"native transcription").await
        .map_err(|error| error.redacted(&redactor))?;
    let text = body.get("text").and_then(Value::as_str)
        .ok_or_else(|| InvokeError::parse(format!("{} response requires a string text field",native.id())))?;
    Ok(TaskOutcome::Done(TaskResult::Transcript {text:text.into(),
        language:language.as_deref().map(str::trim).filter(|language| !language.is_empty()).map(str::to_owned),model:Some(call.model.clone())}))
}

fn audio_extension(mime:&str) -> &'static str {
    match mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase().as_str() {
        "audio/wav"|"audio/x-wav"|"audio/wave" => "wav",
        "audio/mpeg"|"audio/mp3" => "mp3",
        "audio/mp4"|"audio/m4a"|"audio/x-m4a" => "m4a",
        "audio/aac" => "aac","audio/aiff"|"audio/x-aiff" => "aiff",
        "audio/ogg" => "ogg","audio/opus" => "opus",
        "audio/flac"|"audio/x-flac" => "flac","audio/webm" => "webm",_ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{Mock,MockServer,ResponseTemplate};
    use wiremock::matchers::{header,method,path};
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::{AsrRequest,InputAsset};

    fn call(base:&str,native:NativeAsr,language:Option<&str>,prompt:Option<&str>,extra:Value) -> ResolvedCall {
        let (model,endpoint) = match native {
            NativeAsr::MiniMax => ("asr-1.0","/v1/speech_to_text"),
            NativeAsr::Zhipu => ("glm-asr-2512","/api/paas/v4/audio/transcriptions"),
            NativeAsr::SiliconFlow => ("FunAudioLLM/SenseVoiceSmall","/v1/audio/transcriptions"),
        };
        call_with_endpoint(base,model,native.id(),endpoint,TaskRequest::SpeechRecognition(AsrRequest {
            audio:InputAsset {id:None,role:"audio".into(),bytes:b"actual-audio".to_vec(),mime:"audio/wav".into()},language:language.map(str::to_owned),prompt:prompt.map(str::to_owned),extra,
        }))
    }
    fn body(request:&wiremock::Request) -> String {String::from_utf8_lossy(&request.body).into_owned()}

    #[tokio::test]
    async fn minimax_language_is_a_header_and_file_model_auth_cannot_be_overridden() {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/speech_to_text")).and(header("language","zh")).and(header("authorization","Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text":"recognized","duration":0.5}))).expect(1).mount(&server).await;
        let mut call=call(&server.uri(),NativeAsr::MiniMax,Some("zh"),None,json!({"file":"wrong-audio","model":"wrong-model","native_option":"per-call"}));
        call.model_params=json!({"endpoint":"/v1/speech_to_text","language":"en","native_option":"default","temperature":0.2,"api_key":"must-not-leak","headers":{"authorization":"wrong"}});
        let out=MiniMaxSpeechToTextAdapter.submit(&reqwest::Client::new(),&call).await.unwrap();
        assert!(matches!(out,TaskOutcome::Done(TaskResult::Transcript {text,language:Some(language),..}) if text=="recognized" && language=="zh"));
        let requests=server.received_requests().await.unwrap();let sent=body(&requests[0]);
        assert!(sent.contains("name=\"file\"; filename=\"audio.wav\""));assert!(sent.contains("actual-audio"));assert!(!sent.contains("wrong-audio"));assert!(!sent.contains("wrong-model"));
        assert!(sent.contains("name=\"response_format\"\r\n\r\njson"));assert!(sent.contains("name=\"model\"\r\n\r\nasr-1.0"));
        assert!(sent.contains("name=\"native_option\"\r\n\r\nper-call"));assert!(!sent.contains("name=\"language\""));assert!(!sent.contains("must-not-leak"));assert!(!sent.contains("name=\"headers\""));
    }

    #[tokio::test]
    async fn zhipu_prompt_is_native_and_no_openai_language_or_response_format_is_injected() {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/paas/v4/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"glm-asr-2512","text":"recognized"}))).expect(1).mount(&server).await;
        let call=call(&server.uri(),NativeAsr::Zhipu,None,Some("previous transcript"),json!({"prompt":"shadowed","user_id":"native-user","stream":false}));
        ZhipuAudioTranscriptionsAdapter.submit(&reqwest::Client::new(),&call).await.unwrap();
        let requests=server.received_requests().await.unwrap();let sent=body(&requests[0]);
        assert!(sent.contains("name=\"prompt\"\r\n\r\nprevious transcript"));assert!(!sent.contains("shadowed"));assert!(sent.contains("name=\"user_id\"\r\n\r\nnative-user"));
        assert!(!sent.contains("name=\"response_format\""));assert!(!sent.contains("name=\"language\""));assert!(!requests[0].headers.contains_key("language"));
    }

    #[tokio::test]
    async fn siliconflow_sends_only_owned_fields_and_explicit_native_scalar_options() {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text":""}))).expect(1).mount(&server).await;
        let call=call(&server.uri(),NativeAsr::SiliconFlow,None,None,json!({"native_option":7}));
        let out=SiliconFlowAudioTranscriptionsAdapter.submit(&reqwest::Client::new(),&call).await.unwrap();
        assert!(matches!(out,TaskOutcome::Done(TaskResult::Transcript {text,..}) if text.is_empty()));
        let requests=server.received_requests().await.unwrap();let sent=body(&requests[0]);
        assert!(sent.contains("name=\"model\"\r\n\r\nFunAudioLLM/SenseVoiceSmall"));assert!(sent.contains("name=\"native_option\"\r\n\r\n7"));
        for field in ["language","prompt","response_format","stream"] {assert!(!sent.contains(&format!("name=\"{field}\"")));}
    }

    #[tokio::test]
    async fn unsupported_typed_options_and_unsupported_output_modes_fail_before_http() {
        let server=MockServer::start().await;
        for (native,language,prompt,extra) in [
            (NativeAsr::MiniMax,None,Some("context"),json!({})),
            (NativeAsr::Zhipu,Some("zh"),None,json!({})),
            (NativeAsr::SiliconFlow,Some("en"),None,json!({})),
            (NativeAsr::SiliconFlow,None,Some("context"),json!({})),
            (NativeAsr::MiniMax,None,None,json!({"response_format":"srt"})),
            (NativeAsr::MiniMax,None,None,json!({"stream":true})),
            (NativeAsr::Zhipu,None,None,json!({"stream":true})),
            (NativeAsr::Zhipu,None,None,json!({"response_format":"json"})),
            (NativeAsr::Zhipu,None,None,json!({"hotwords":["unsupported invented encoding"]})),
        ] {
            let error=submit(native,&reqwest::Client::new(),&call(&server.uri(),native,language,prompt,extra)).await.unwrap_err();
            assert_eq!(error.kind,InvokeErrorKind::InvalidParams,"{}",native.id());
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_invalid_and_malformed_text_never_become_empty_successes() {
        for native in [NativeAsr::MiniMax,NativeAsr::Zhipu,NativeAsr::SiliconFlow] {
            for reply in ["{}","{\"text\":null}","{\"text\":7}","not-json-sk-test"] {
                let server=MockServer::start().await;
                Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_raw(reply,"application/json")).expect(1).mount(&server).await;
                let error=submit(native,&reqwest::Client::new(),&call(&server.uri(),native,None,None,json!({}))).await.unwrap_err();
                assert_eq!(error.kind,InvokeErrorKind::ParseError);assert!(!error.message.contains("sk-test"));
            }
        }
    }

    #[tokio::test]
    async fn authentication_errors_use_shared_redaction_for_every_native_protocol() {
        for native in [NativeAsr::MiniMax,NativeAsr::Zhipu,NativeAsr::SiliconFlow] {
            let server=MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":{"message":"rejected sk-test"}}))).expect(1).mount(&server).await;
            let error=submit(native,&reqwest::Client::new(),&call(&server.uri(),native,None,None,json!({}))).await.unwrap_err();
            assert_eq!(error.kind,InvokeErrorKind::Auth);assert_eq!(error.http_status,Some(401));assert!(!error.message.contains("sk-test"));
        }
    }

    #[tokio::test]
    async fn response_size_is_bounded_and_language_cannot_inject_http_headers() {
        let server=MockServer::start().await;
        let error=submit(NativeAsr::MiniMax,&reqwest::Client::new(),&call(&server.uri(),NativeAsr::MiniMax,Some("en\r\nx-unsafe: true"),None,json!({}))).await.unwrap_err();
        assert_eq!(error.kind,InvokeErrorKind::InvalidParams);assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_raw(" ".repeat(MAX_TRANSCRIPTION_RESPONSE_BYTES as usize+1),"application/json")).expect(1).mount(&server).await;
        let error=submit(NativeAsr::SiliconFlow,&reqwest::Client::new(),&call(&server.uri(),NativeAsr::SiliconFlow,None,None,json!({}))).await.unwrap_err();
        assert_eq!(error.kind,InvokeErrorKind::ProviderError);assert!(error.message.contains("cap"));
    }
}
