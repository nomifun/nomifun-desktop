//! StepFun Realtime JSON is confined to this integration module.
//! Sources: platform.stepfun.com/docs/zh/{api-reference/realtime/chat,guides/developer/realtime}.

use super::wire::{WireEvent, decode_audio, required_string, string};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepFunConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub allow_cross_origin_credentials: bool,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub vad: StepFunVad,
    #[serde(default)]
    pub max_response_output_tokens: Option<u32>,
}

impl Default for StepFunConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            allow_cross_origin_credentials: false,
            voice: None,
            vad: StepFunVad::default(),
            max_response_output_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StepFunVad {
    Disabled,
    Server {
        prefix_padding_ms: u32,
        silence_duration_ms: u32,
        energy_awakeness_threshold: u32,
    },
}

impl Default for StepFunVad {
    fn default() -> Self {
        // Explicitly set every VAD field; the API and developer guide disagree
        // about the default enabled state.
        Self::Server {
            prefix_padding_ms: 500,
            silence_duration_ms: 100,
            energy_awakeness_threshold: 2500,
        }
    }
}

impl StepFunConfig {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "invalid StepFun voice configuration".to_string())?;
        if config
            .voice
            .as_ref()
            .is_some_and(|voice| voice.trim().is_empty())
        {
            return Err("StepFun voice must be a non-empty ID".into());
        }
        if config.max_response_output_tokens == Some(0) {
            return Err("StepFun max_response_output_tokens must be positive".into());
        }
        if matches!(config.vad, StepFunVad::Disabled) {
            return Err("native full-duplex voice requires explicit server VAD; manual buffer commit mode is not this interaction route".into());
        }
        if let StepFunVad::Server {
            energy_awakeness_threshold,
            silence_duration_ms,
            ..
        } = &config.vad
        {
            if *energy_awakeness_threshold > 5000 || *silence_duration_ms == 0 {
                return Err("StepFun VAD thresholds are outside the supported range".into());
            }
        }
        Ok(config)
    }

    pub fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"properties":{
            "endpoint":{"type":["string","null"],"minLength":1},
            "allow_cross_origin_credentials":{"type":"boolean"},
            "voice":{"type":["string","null"],"minLength":1},
            "max_response_output_tokens":{"type":["integer","null"],"minimum":1},
            "vad":{"oneOf":[
                {"type":"object","additionalProperties":false,"required":["mode","prefix_padding_ms","silence_duration_ms","energy_awakeness_threshold"],"properties":{
                    "mode":{"const":"server"},"prefix_padding_ms":{"type":"integer","minimum":0},
                    "silence_duration_ms":{"type":"integer","minimum":1},"energy_awakeness_threshold":{"type":"integer","minimum":0,"maximum":5000}}}
            ]}
        }})
    }

    pub fn session_update(&self, instructions: &str, tools: Value, event_id: &str) -> Value {
        let vad = match &self.vad {
            StepFunVad::Disabled => Value::Null,
            StepFunVad::Server {
                prefix_padding_ms,
                silence_duration_ms,
                energy_awakeness_threshold,
            } => json!({
                "type":"server_vad", "prefix_padding_ms":prefix_padding_ms,
                "silence_duration_ms":silence_duration_ms,"energy_awakeness_threshold":energy_awakeness_threshold
            }),
        };
        let mut session = json!({"modalities":["text","audio"],"instructions":instructions,
            "input_audio_format":"pcm16","output_audio_format":"pcm16","turn_detection":vad,"tools":tools});
        if let Some(voice) = &self.voice {
            session["voice"] = json!(voice);
        }
        if let Some(limit) = self.max_response_output_tokens {
            session["max_response_output_tokens"] = json!(limit);
        }
        json!({"type":"session.update","event_id":event_id,"session":session})
    }
}

#[derive(Default)]
pub(super) struct StepFunDecoder {
    calls: BTreeMap<String, String>,
    emitted_calls: BTreeSet<String>,
    session_configuration: Value,
}

impl StepFunDecoder {
    pub fn decode(
        &mut self,
        raw: &Value,
        max_audio_bytes: usize,
    ) -> Result<Vec<WireEvent>, String> {
        let kind = required_string(raw, "type")?;
        if self.calls.len() > 4096 || self.emitted_calls.len() > 4096 {
            return Err("StepFun call identity retention limit reached".into());
        }
        let response_id = string(raw, "response_id")
            .or_else(|| raw.get("response").and_then(|r| string(r, "id")));
        let item_id =
            string(raw, "item_id").or_else(|| raw.get("item").and_then(|r| string(r, "id")));
        let event = match kind.as_str() {
            "session.created" => {
                self.session_configuration = raw.get("session").cloned().unwrap_or(Value::Null);
                None
            }
            "session.updated" => {
                let updated = raw
                    .get("session")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        "StepFun configuration acknowledgment has no session".to_string()
                    })?;
                let mut configuration = self
                    .session_configuration
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                configuration.extend(updated.clone());
                self.session_configuration = Value::Object(configuration);
                Some(WireEvent::Ready {
                    session_id: required_string(&self.session_configuration, "id")?,
                    configuration: self.session_configuration.clone(),
                })
            }
            "input_audio_buffer.speech_started" => Some(WireEvent::Speech { active: true }),
            "input_audio_buffer.speech_stopped" => Some(WireEvent::Speech { active: false }),
            "input_audio_buffer.cleared" => Some(WireEvent::InputCleared {
                client_event_id: string(raw, "client_event_id"),
            }),
            "response.created" => Some(WireEvent::OutputStarted {
                response_id: response_id
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "StepFun response has no ID".to_string())?,
            }),
            "response.audio.delta" => Some(WireEvent::Audio {
                response_id: Some(
                    response_id
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| "StepFun audio has no response fence".to_string())?,
                ),
                item_id,
                bytes: decode_audio(raw, "delta", max_audio_bytes, 2)?,
                start_ms: None,
                end_ms: None,
            }),
            "response.audio.done" => Some(WireEvent::AudioDone {
                response_id,
                item_id,
            }),
            "response.audio_transcript.delta"
            | "response.audio_transcript.done"
            | "conversation.item.input_audio_transcription.completed" => {
                let user = kind == "conversation.item.input_audio_transcription.completed";
                let complete = !kind.ends_with(".delta");
                Some(WireEvent::Transcript {
                    user,
                    response_id: if user { None } else { response_id.clone() },
                    fragment_id: item_id.or(response_id).unwrap_or_else(|| kind.clone()),
                    text: string(raw, if complete { "transcript" } else { "delta" })
                        .unwrap_or_default(),
                    complete,
                    append: !complete,
                    start_ms: None,
                    end_ms: None,
                })
            }
            "conversation.item.created" => {
                if let Some(item) = raw
                    .get("item")
                    .filter(|item| string(item, "type").as_deref() == Some("function_call"))
                {
                    let id = required_string(item, "call_id")?;
                    let name = required_string(item, "name")?;
                    self.calls.insert(id, name);
                }
                raw.get("item")
                    .filter(|item| string(item, "type").as_deref() == Some("message"))
                    .cloned()
                    .map(|item| WireEvent::MessageAccepted { item })
            }
            "response.function_call_arguments.done" => {
                let id = required_string(raw, "call_id")?;
                let name = string(raw, "name")
                    .or_else(|| self.calls.get(&id).cloned())
                    .ok_or_else(|| "StepFun completed function call has no name".to_string())?;
                self.completed_call(id, name, raw.get("arguments"), response_id)?
            }
            "response.output_item.done" => {
                if let Some(item) = raw.get("item").filter(|item| {
                    string(item, "type").as_deref() == Some("function_call")
                        && string(item, "status").as_deref() == Some("completed")
                }) {
                    self.completed_call(
                        required_string(item, "call_id")?,
                        required_string(item, "name")?,
                        item.get("arguments"),
                        response_id,
                    )?
                } else {
                    None
                }
            }
            "response.done" => {
                let response = raw.get("response").unwrap_or(&Value::Null);
                let mut events = Vec::new();
                if string(response, "status").as_deref() == Some("completed") {
                    if let Some(items) = response.get("output").and_then(Value::as_array) {
                        for item in items
                            .iter()
                            .filter(|item| string(item, "type").as_deref() == Some("function_call"))
                        {
                            if let Some(event) = self.completed_call(
                                required_string(item, "call_id")?,
                                required_string(item, "name")?,
                                item.get("arguments"),
                                response_id.clone(),
                            )? {
                                events.push(event);
                            }
                        }
                    }
                    events.push(WireEvent::AudioDone {
                        response_id,
                        item_id: None,
                    });
                } else if matches!(
                    string(response, "status").as_deref(),
                    Some("cancelled" | "interrupted")
                ) {
                    events.push(WireEvent::OutputInterrupted { response_id });
                }
                return Ok(events);
            }
            "response.cancelled" => Some(WireEvent::OutputInterrupted { response_id }),
            "error" => Some(WireEvent::Error {
                code: raw.get("error").and_then(|e| string(e, "code")),
                message: raw
                    .get("error")
                    .and_then(|e| string(e, "message"))
                    .unwrap_or_else(|| "StepFun voice error".into()),
            }),
            // Unknown raw events never enter product projections or diagnostics.
            _ => None,
        };
        Ok(event.into_iter().collect())
    }

    fn completed_call(
        &mut self,
        id: String,
        name: String,
        arguments: Option<&Value>,
        response_id: Option<String>,
    ) -> Result<Option<WireEvent>, String> {
        if self.emitted_calls.contains(&id) {
            return Ok(None);
        }
        let encoded = arguments.and_then(Value::as_str).ok_or_else(|| {
            "StepFun completed tool call requires full JSON arguments".to_string()
        })?;
        let arguments: Value = serde_json::from_str(encoded)
            .map_err(|_| "StepFun completed tool call arguments are invalid JSON".to_string())?;
        if !arguments.is_object() {
            return Err("StepFun tool call arguments must be an object".into());
        }
        self.emitted_calls.insert(id.clone());
        self.calls.remove(&id);
        Ok(Some(WireEvent::ToolCall {
            call_id: id,
            response_id,
            name,
            arguments,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_clear_ack_is_distinct_from_a_vad_or_transcript_event() {
        assert_eq!(
            StepFunDecoder::default()
                .decode(
                    &json!({"type":"input_audio_buffer.cleared","event_id":"server-clear-event"}),
                    48000
                )
                .unwrap(),
            vec![WireEvent::InputCleared {
                client_event_id: None
            }]
        );
    }
    #[test]
    fn vad_is_explicit_and_extra_wire_is_rejected() {
        assert!(StepFunConfig::parse(&json!({"extra":{"tools":[]}})).is_err());
        assert!(StepFunConfig::parse(&json!({"vad":{"mode":"disabled"}})).is_err());
        let config = StepFunConfig {
            vad: StepFunVad::Disabled,
            ..StepFunConfig::default()
        };
        assert!(config.session_update("", json!([]), "1")["session"]["turn_detection"].is_null());
        assert_eq!(
            StepFunConfig::default().session_update("", json!([]), "1")["session"]["turn_detection"]
                ["type"],
            "server_vad"
        );
    }
    #[test]
    fn incomplete_tools_never_trigger_and_complete_call_is_deduplicated() {
        let mut decoder = StepFunDecoder::default();
        assert!(decoder.decode(&json!({"type":"response.function_call_arguments.delta","call_id":"a","arguments":"{"}),48000).unwrap().is_empty());
        let done = json!({"type":"response.function_call_arguments.done","call_id":"a","name":"start","arguments":"{\"text\":\"work\"}"});
        assert!(matches!(
            decoder.decode(&done, 48000).unwrap().as_slice(),
            [WireEvent::ToolCall { .. }]
        ));
        assert!(decoder.decode(&done, 48000).unwrap().is_empty());
    }
    #[test]
    fn malformed_completed_tool_cannot_execute() {
        let mut decoder = StepFunDecoder::default();
        assert!(decoder.decode(&json!({"type":"response.function_call_arguments.done","call_id":"a","name":"start","arguments":"{"}),48000).is_err());
    }
}
