//! MiniMax's real music composition API, separate from speech synthesis.
//! Contract: https://platform.minimax.io/docs/api-reference/music-generation
//! Non-streaming hex output is materialized before provider URLs can expire.
//! The provider currently restricts this API to existing paying Music users;
//! account refusals remain errors rather than falling back to TTS.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_hex, error_from_response, read_body_capped, send_with_rotation, MAX_ARTIFACT_BYTES};
use crate::types::{ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult};

use super::json_request_body;

pub struct MiniMaxMusicAdapter;

#[async_trait]
impl ProtocolAdapter for MiniMaxMusicAdapter {
    fn id(&self) -> &'static str { "minimax.music" }

    fn supports(&self, task: ModelTask) -> bool { task == ModelTask::MusicGeneration }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::MusicGeneration(req) = &call.request else {
            return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "minimax.music only composes music"));
        };
        let prompt_len = req.prompt.chars().count();
        if prompt_len == 0 || prompt_len > 2000 {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "Music prompt must contain 1–2000 characters"));
        }
        let lyrics = req.lyrics.as_deref().filter(|value| !value.trim().is_empty());
        if lyrics.is_some_and(|value| value.chars().count() > 3500) {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "Music lyrics exceed 3500 characters"));
        }
        if req.instrumental && lyrics.is_some() {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "Instrumental music cannot contain lyrics"));
        }
        let format = req.format.as_deref().unwrap_or("mp3");
        // The documented composition endpoint guarantees MP3. Additional
        // codecs must be verified independently rather than inherited from TTS.
        if format != "mp3" {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "minimax.music currently supports mp3 output"));
        }
        let mut body = json!({
            "model": call.model,
            "prompt": req.prompt,
            "is_instrumental": req.instrumental,
            "lyrics_optimizer": !req.instrumental && lyrics.is_none(),
            "stream": false,
            "output_format": "hex",
            "audio_setting": { "format": format, "sample_rate": 44100, "bitrate": 256000 },
        });
        if let Some(lyrics) = lyrics { body["lyrics"] = json!(lyrics); }
        let body = json_request_body(&call.model_params, &req.extra, body)?;
        // Duration is an output property of this provider, not a supported
        // request setting. Do not silently drop it or claim it was respected.
        if ["seconds", "duration", "duration_ms", "count", "quality", "resolution"]
            .iter().any(|key| body.get(key).is_some()) {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "minimax.music does not support requested duration, count, quality, or resolution controls"));
        }
        let url = call.endpoint_url()?;
        let response = send_with_rotation(&call.connection.auth, || {
            Ok(http.post(&url).timeout(Duration::from_secs(300)).json(&body))
        }).await?;
        if !response.status().is_success() { return Err(error_from_response(response).await); }
        let bytes = read_body_capped(response, MAX_ARTIFACT_BYTES).await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| InvokeError::parse(format!("invalid minimax music JSON: {error}")))?;
        let audio = parse_music_audio(&value)?;
        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(audio), mime: Some("audio/mpeg".to_owned()),
        }])))
    }
}

fn parse_music_audio(value: &Value) -> Result<Vec<u8>, InvokeError> {
    let status = value.pointer("/base_resp/status_code").and_then(Value::as_i64)
        .ok_or_else(|| InvokeError::parse("minimax music response has no base_resp.status_code"))?;
    if status != 0 {
        let message = value.pointer("/base_resp/status_msg").and_then(Value::as_str).unwrap_or("music generation refused");
        return Err(InvokeError::new(InvokeErrorKind::ProviderError, format!("minimax music failed ({status}): {message}")));
    }
    if value.pointer("/data/status").and_then(Value::as_i64) != Some(2) {
        return Err(InvokeError::parse("minimax music did not return a complete track"));
    }
    let audio = value.pointer("/data/audio").and_then(Value::as_str)
        .ok_or_else(|| InvokeError::parse("minimax music returned no audio"))?;
    decode_hex(audio).filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| InvokeError::parse("minimax music returned empty or invalid hex audio"))
}

#[cfg(test)]
mod tests {
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::MusicGenRequest;

    fn request() -> MusicGenRequest {
        MusicGenRequest { prompt: "Calm acoustic guitar".into(), lyrics: None, instrumental: true, format: None, extra: json!({}) }
    }
    fn call(base: &str, request: MusicGenRequest) -> ResolvedCall {
        call_with_endpoint(base, "music-3.0", "minimax.music", "/music_generation", TaskRequest::MusicGeneration(request))
    }

    #[tokio::test]
    async fn composes_a_real_instrumental_request_and_decodes_completed_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/music_generation"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({"model":"music-3.0", "is_instrumental":true, "stream":false, "output_format":"hex", "audio_setting":{"format":"mp3"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"base_resp":{"status_code":0},"data":{"status":2,"audio":"494433"}})))
            .expect(1).mount(&server).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = MiniMaxMusicAdapter.submit(&http, &call(&server.uri(), request())).await.unwrap() else { panic!("expected audio") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"ID3"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));
    }

    #[tokio::test]
    async fn vocals_without_supplied_lyrics_use_documented_lyrics_optimizer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"is_instrumental":false,"lyrics_optimizer":true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"base_resp":{"status_code":0},"data":{"status":2,"audio":"494433"}})))
            .expect(1).mount(&server).await;
        let mut req = request(); req.instrumental = false;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        MiniMaxMusicAdapter.submit(&http, &call(&server.uri(), req)).await.unwrap();
    }

    #[tokio::test]
    async fn unsupported_duration_fails_before_any_provider_request() {
        let server = MockServer::start().await;
        let mut req = request(); req.extra = json!({"seconds":30});
        let error = MiniMaxMusicAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), req)).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn refusal_partial_and_malformed_audio_never_become_success() {
        let refused = json!({"base_resp":{"status_code":1004,"status_msg":"account unavailable"},"data":{"status":2,"audio":"494433"}});
        assert_eq!(parse_music_audio(&refused).unwrap_err().kind, InvokeErrorKind::ProviderError);
        for data in [json!({"status":1,"audio":"494433"}), json!({"status":2,"audio":"zz"}), json!({"status":2,"audio":""})] {
            assert_eq!(parse_music_audio(&json!({"base_resp":{"status_code":0},"data":data})).unwrap_err().kind, InvokeErrorKind::ParseError);
        }
    }
}
