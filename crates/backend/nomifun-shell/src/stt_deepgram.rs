use nomifun_api_types::{DeepgramSpeechToTextConfig, SpeechToTextProvider, SpeechToTextResult};
use reqwest::Client;

use crate::error::SttError;

const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";

pub async fn transcribe(
    client: &Client,
    config: &DeepgramSpeechToTextConfig,
    audio_data: Vec<u8>,
    mime_type: &str,
    language_hint: Option<&str>,
) -> Result<SpeechToTextResult, SttError> {
    if config.api_key.is_empty() {
        return Err(SttError::DeepgramNotConfigured);
    }

    let base_url = config
        .base_url
        .as_deref()
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let mut query_params = vec![("model", config.model.clone())];

    let language = language_hint.or(config.language.as_deref()).filter(|s| !s.is_empty());
    if let Some(lang) = language {
        query_params.push(("language", lang.to_owned()));
    } else if config.detect_language == Some(true) {
        query_params.push(("detect_language", "true".to_owned()));
    }

    if config.punctuate == Some(true) {
        query_params.push(("punctuate", "true".to_owned()));
    }

    if config.smart_format == Some(true) {
        query_params.push(("smart_format", "true".to_owned()));
    }

    let url = format!("{base_url}/v1/listen");

    let response = client
        .post(&url)
        .header("Authorization", format!("Token {}", config.api_key))
        .header("Content-Type", mime_type)
        .query(&query_params)
        .body(audio_data)
        .send()
        .await
        .map_err(|e| SttError::RequestFailed(format!("Deepgram request error: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "<unreadable>".to_owned());
        return Err(SttError::RequestFailed(format!(
            "Deepgram API returned {status}: {body}"
        )));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| SttError::RequestFailed(format!("failed to parse Deepgram response: {e}")))?;

    let transcript = body["results"]["channels"]
        .get(0)
        .and_then(|ch| ch["alternatives"].get(0))
        .and_then(|alt| alt["transcript"].as_str())
        .unwrap_or("")
        .to_owned();

    let detected_language = body["results"]["channels"]
        .get(0)
        .and_then(|ch| ch["detected_language"].as_str())
        .map(|s| s.to_owned())
        .or_else(|| language.map(|s| s.to_owned()));

    let model_name = extract_model_name(&body).unwrap_or_else(|| config.model.clone());

    Ok(SpeechToTextResult {
        text: transcript,
        model: model_name,
        provider: SpeechToTextProvider::Deepgram,
        language: detected_language,
    })
}

fn extract_model_name(body: &serde_json::Value) -> Option<String> {
    body["metadata"]["model_info"]
        .as_object()
        .and_then(|map| map.values().next())
        .and_then(|info| info["name"].as_str())
        .map(|s| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_url_value() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.deepgram.com");
    }

    #[tokio::test]
    async fn empty_api_key_returns_not_configured() {
        let config = DeepgramSpeechToTextConfig {
            api_key: String::new(),
            base_url: None,
            model: "nova-2".into(),
            language: None,
            detect_language: None,
            punctuate: None,
            smart_format: None,
        };
        let result = transcribe(&Client::new(), &config, vec![0u8; 10], "audio/wav", None).await;
        assert!(matches!(result, Err(SttError::DeepgramNotConfigured)));
    }

    #[test]
    fn extract_model_name_from_response() {
        let body = serde_json::json!({
            "metadata": {
                "model_info": {
                    "some-uuid": {
                        "name": "2-general-nova",
                        "version": "2024-01-18.26916"
                    }
                }
            },
            "results": {
                "channels": [{
                    "alternatives": [{ "transcript": "hello" }]
                }]
            }
        });
        assert_eq!(extract_model_name(&body), Some("2-general-nova".to_owned()));
    }

    #[test]
    fn extract_model_name_missing_metadata() {
        let body = serde_json::json!({
            "results": {
                "channels": [{ "alternatives": [{ "transcript": "hi" }] }]
            }
        });
        assert_eq!(extract_model_name(&body), None);
    }

    #[test]
    fn extract_model_name_empty_model_info() {
        let body = serde_json::json!({
            "metadata": { "model_info": {} },
            "results": {
                "channels": [{ "alternatives": [{ "transcript": "hi" }] }]
            }
        });
        assert_eq!(extract_model_name(&body), None);
    }
}
