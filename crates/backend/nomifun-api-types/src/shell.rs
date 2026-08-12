use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Shell operation types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    Vscode,
    Terminal,
    Explorer,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenFileRequest {
    pub file_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowItemInFolderRequest {
    pub file_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenExternalRequest {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckToolInstalledRequest {
    pub tool: ToolType,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckToolInstalledResponse {
    pub installed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenFolderWithRequest {
    pub folder_path: String,
    pub tool: ToolType,
}

// ---------------------------------------------------------------------------
// Text-to-speech types
// ---------------------------------------------------------------------------

/// `POST /api/tts` request: synthesize `text` on `(provider_id, model)`.
/// `voice`/`format` are optional passthroughs to the provider (the adapter
/// applies its own defaults).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsApiRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub text: String,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

/// Preference key holding the install-wide speech-synthesis default.
/// Deliberately parallel to `tools.speechToText`, minus the `enabled` switch:
/// speech synthesis has no input-box affordance to gate, so the key's PRESENCE
/// is the configuration and a second boolean would only be able to disagree
/// with it.
pub const TEXT_TO_SPEECH_PREFERENCE_KEY: &str = "tools.textToSpeech";

/// The install-wide speech-synthesis default: which catalog model speaks and in
/// which provider voice. Every companion whose `voice.tts` slot is empty falls
/// back to this.
///
/// There is no un-namespaced twin: provider credentials and transport belong
/// exclusively to the provider/model capability catalog.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextToSpeechConfig {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    /// Provider voice id (free text). `None` = the provider's own default voice.
    #[serde(default)]
    pub voice: Option<String>,
}

impl TextToSpeechConfig {
    /// Read the global default out of a preferences snapshot. A missing or
    /// malformed value answers `None` — "no global default" — because a caller
    /// that cannot synthesize must say so, not fail the whole request.
    pub fn from_preferences(prefs: &crate::ClientPreferencesResponse) -> Option<Self> {
        prefs
            .get(TEXT_TO_SPEECH_PREFERENCE_KEY)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}

// ---------------------------------------------------------------------------
// Speech-to-text types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SpeechToTextResult {
    pub text: String,
    pub model: String,
    /// Resolved provider platform identifier, for display/diagnostics only.
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpeechToTextConfig {
    pub enabled: bool,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub language: Option<String>,
    pub auto_send: Option<bool>,
}

impl<'de> Deserialize<'de> for SpeechToTextConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            enabled: bool,
            #[serde(
                default,
                deserialize_with = "crate::serde_util::deserialize_optional_provider_id"
            )]
            provider_id: Option<String>,
            #[serde(
                default,
                deserialize_with = "crate::serde_util::deserialize_optional_model_name"
            )]
            model: Option<String>,
            #[serde(default)]
            language: Option<String>,
            #[serde(default)]
            auto_send: Option<bool>,
        }

        let wire = Wire::deserialize(deserializer)?;
        crate::serde_util::validate_optional_provider_model_pair(
            wire.provider_id.as_deref(),
            wire.model.as_deref(),
        )
        .map_err(serde::de::Error::custom)?;
        Ok(Self {
            enabled: wire.enabled,
            provider_id: wire.provider_id,
            model: wire.model,
            language: wire.language,
            auto_send: wire.auto_send,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientPreferencesResponse;
    use serde_json::json;

    // -- ToolType --

    #[test]
    fn tool_type_serializes_lowercase() {
        assert_eq!(serde_json::to_value(ToolType::Vscode).unwrap(), "vscode");
        assert_eq!(
            serde_json::to_value(ToolType::Terminal).unwrap(),
            "terminal"
        );
        assert_eq!(
            serde_json::to_value(ToolType::Explorer).unwrap(),
            "explorer"
        );
    }

    #[test]
    fn tool_type_deserializes_lowercase() {
        let v: ToolType = serde_json::from_str(r#""vscode""#).unwrap();
        assert_eq!(v, ToolType::Vscode);
        let t: ToolType = serde_json::from_str(r#""terminal""#).unwrap();
        assert_eq!(t, ToolType::Terminal);
        let e: ToolType = serde_json::from_str(r#""explorer""#).unwrap();
        assert_eq!(e, ToolType::Explorer);
    }

    #[test]
    fn tool_type_rejects_unknown() {
        let result = serde_json::from_str::<ToolType>(r#""unknown""#);
        assert!(result.is_err());
    }

    // -- Shell request types --

    #[test]
    fn open_file_request_snake_case() {
        let raw = json!({ "file_path": "/tmp/test.txt" });
        let req: OpenFileRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.file_path, "/tmp/test.txt");
    }

    #[test]
    fn open_file_request_missing_field() {
        let result = serde_json::from_value::<OpenFileRequest>(json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn show_item_in_folder_request_snake_case() {
        let raw = json!({ "file_path": "/home/user/doc.pdf" });
        let req: ShowItemInFolderRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.file_path, "/home/user/doc.pdf");
    }

    #[test]
    fn open_external_request_parses() {
        let raw = json!({ "url": "https://example.com" });
        let req: OpenExternalRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.url, "https://example.com");
    }

    #[test]
    fn check_tool_installed_request_parses() {
        let raw = json!({ "tool": "vscode" });
        let req: CheckToolInstalledRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.tool, ToolType::Vscode);
    }

    #[test]
    fn check_tool_installed_response_serializes() {
        let resp = CheckToolInstalledResponse { installed: true };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["installed"], true);
    }

    #[test]
    fn open_folder_with_request_snake_case() {
        let raw = json!({ "folder_path": "/tmp", "tool": "terminal" });
        let req: OpenFolderWithRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.folder_path, "/tmp");
        assert_eq!(req.tool, ToolType::Terminal);
    }

    // -- TtsApiRequest --

    #[test]
    fn tts_request_full_parses() {
        let raw = json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "tts-1",
            "text": "hello",
            "voice": "nova",
            "format": "wav"
        });
        let req: TtsApiRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.provider_id, "018f0000-0000-7000-8000-000000000001");
        assert_eq!(req.model, "tts-1");
        assert_eq!(req.text, "hello");
        assert_eq!(req.voice.as_deref(), Some("nova"));
        assert_eq!(req.format.as_deref(), Some("wav"));
    }

    #[test]
    fn tts_request_minimal_defaults_optionals() {
        let raw = json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "tts-1",
            "text": "hi"
        });
        let req: TtsApiRequest = serde_json::from_value(raw).unwrap();
        assert!(req.voice.is_none());
        assert!(req.format.is_none());
    }

    #[test]
    fn tts_request_rejects_invalid_provider_id() {
        let raw = json!({
            "provider_id": "not-a-uuid",
            "model": "tts-1",
            "text": "hi"
        });
        assert!(serde_json::from_value::<TtsApiRequest>(raw).is_err());
    }

    #[test]
    fn tts_request_rejects_untrimmed_model() {
        let raw = json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": " tts-1 ",
            "text": "hi"
        });
        assert!(serde_json::from_value::<TtsApiRequest>(raw).is_err());
    }

    #[test]
    fn tts_request_rejects_unknown_fields() {
        let raw = json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "tts-1",
            "text": "hi",
            "speed": 1.5
        });
        assert!(serde_json::from_value::<TtsApiRequest>(raw).is_err());
    }

    #[test]
    fn tts_request_missing_text_is_error() {
        let raw = json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "tts-1"
        });
        assert!(serde_json::from_value::<TtsApiRequest>(raw).is_err());
    }

    // -- SpeechToTextResult --

    #[test]
    fn stt_result_serializes_with_language() {
        let result = SpeechToTextResult {
            text: "hello world".to_owned(),
            model: "whisper-1".to_owned(),
            provider: "stepfun".to_owned(),
            language: Some("en".to_owned()),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["text"], "hello world");
        assert_eq!(json["model"], "whisper-1");
        assert_eq!(json["provider"], "stepfun");
        assert_eq!(json["language"], "en");
    }

    #[test]
    fn stt_result_omits_null_language() {
        let result = SpeechToTextResult {
            text: "test".to_owned(),
            model: "nova-2".to_owned(),
            provider: "deepgram".to_owned(),
            language: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("language").is_none());
    }

    // -- SpeechToTextConfig --

    #[test]
    fn stt_config_uses_only_catalog_coordinates() {
        let raw = json!({
            "enabled": true,
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "step-asr",
            "language": "zh",
            "auto_send": true,
        });
        let config: SpeechToTextConfig = serde_json::from_value(raw).unwrap();
        assert!(config.enabled);
        assert_eq!(
            config.provider_id.as_deref(),
            Some("018f0000-0000-7000-8000-000000000001")
        );
        assert_eq!(config.model.as_deref(), Some("step-asr"));
        assert_eq!(config.language.as_deref(), Some("zh"));
        assert_eq!(config.auto_send, Some(true));
    }

    #[test]
    fn stt_config_minimal() {
        let raw = json!({
            "enabled": false
        });
        let config: SpeechToTextConfig = serde_json::from_value(raw).unwrap();
        assert!(!config.enabled);
        assert!(config.provider_id.is_none());
        assert!(config.model.is_none());
        assert!(config.auto_send.is_none());
    }

    #[test]
    fn stt_config_rejects_credentials_and_provider_guess() {
        for invalid in [
            json!({"enabled": true, "provider": "openai"}),
            json!({"enabled": true, "openai": {"api_key": "secret"}}),
            json!({"enabled": true, "deepgram": {"api_key": "secret"}}),
        ] {
            assert!(serde_json::from_value::<SpeechToTextConfig>(invalid).is_err());
        }
    }

    // -- TextToSpeechConfig --

    #[test]
    fn text_to_speech_config_reads_the_tools_preference_key() {
        let provider_id = "0190f5fe-7c00-7a00-8000-0000000000aa";
        let prefs = ClientPreferencesResponse::from([(
            TEXT_TO_SPEECH_PREFERENCE_KEY.to_owned(),
            json!({ "provider_id": provider_id, "model": "tts-1", "voice": "alloy" }),
        )]);
        let config = TextToSpeechConfig::from_preferences(&prefs).unwrap();
        assert_eq!(config.provider_id, provider_id);
        assert_eq!(config.model, "tts-1");
        assert_eq!(config.voice.as_deref(), Some("alloy"));
    }

    #[test]
    fn text_to_speech_config_has_no_enabled_switch_and_fails_closed() {
        let provider_id = "0190f5fe-7c00-7a00-8000-0000000000aa";
        // Presence of the key IS the configuration — an `enabled` field would be
        // a second source of truth and is rejected outright.
        assert!(
            serde_json::from_value::<TextToSpeechConfig>(json!({
                "provider_id": provider_id,
                "model": "tts-1",
                "voice": null,
                "enabled": true
            }))
            .is_err()
        );
        for invalid in [
            json!({ "provider_id": "prov_legacy", "model": "tts-1", "voice": null }),
            json!({ "provider_id": provider_id, "model": " ", "voice": null }),
            json!({ "model": "tts-1", "voice": null }),
        ] {
            assert!(serde_json::from_value::<TextToSpeechConfig>(invalid).is_err());
        }
        // An absent or malformed preference is "no global default", never a panic.
        assert!(TextToSpeechConfig::from_preferences(&ClientPreferencesResponse::new()).is_none());
        let broken = ClientPreferencesResponse::from([(
            TEXT_TO_SPEECH_PREFERENCE_KEY.to_owned(),
            json!("nonsense"),
        )]);
        assert!(TextToSpeechConfig::from_preferences(&broken).is_none());
    }
}
