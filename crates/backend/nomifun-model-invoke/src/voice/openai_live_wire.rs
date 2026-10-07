//! GPT-Live is a distinct protocol, not the OpenAI Realtime API.
//! Sources: developers.openai.com/api/docs/guides/{live-delegation,voice-websockets,voice-webrtc,voice-server-controls}.

use super::wire::{WireEvent, decode_audio, required_string, string};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum LiveFormat {
    #[default]
    Pcm24k,
    Pcm16k,
    Pcmu8k,
    Pcma8k,
}

impl LiveFormat {
    pub fn wire(self) -> Value {
        match self {
            Self::Pcm24k => json!({"type":"audio/pcm","rate":24000}),
            Self::Pcm16k => json!({"type":"audio/pcm","rate":16000}),
            Self::Pcmu8k => json!({"type":"audio/pcmu","rate":8000}),
            Self::Pcma8k => json!({"type":"audio/pcma","rate":8000}),
        }
    }
    pub fn sample_bytes(self) -> usize {
        if matches!(self, Self::Pcm24k | Self::Pcm16k) {
            2
        } else {
            1
        }
    }
    pub fn rate(self) -> u32 {
        match self {
            Self::Pcm24k => 24000,
            Self::Pcm16k => 16000,
            _ => 8000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct LiveConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub allow_cross_origin_credentials: bool,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub format: LiveFormat,
}

impl LiveConfig {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "invalid GPT-Live voice configuration".to_string())?;
        if config
            .voice
            .as_ref()
            .is_some_and(|voice| voice.trim().is_empty())
        {
            return Err("GPT-Live voice ID must not be empty".into());
        }
        if matches!(config.format, LiveFormat::Pcmu8k | LiveFormat::Pcma8k) {
            return Err("GPT-Live G.711 requires an encoded-media endpoint; choose a supported PCM relay format".into());
        }
        Ok(config)
    }
    pub fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"properties":{
            "endpoint":{"type":["string","null"],"minLength":1},
            "allow_cross_origin_credentials":{"type":"boolean"},
            "voice":{"type":["string","null"],"minLength":1},
            "format":{"type":"string","enum":["pcm24k","pcm16k"]}
        }})
    }
    pub fn session(&self, model: &str, instructions: &str, history: Value, native: bool) -> Value {
        let mut session = json!({"model":model,"instructions":instructions,"input":history,"store":false,"delegation":{"type":"client"}});
        if !native {
            session["audio"]["format"] = self.format.wire();
        } else {
            session["client"] =
                json!({"data_channel":{"allowed_client_events":[],"allowed_server_events":[]}});
        }
        if let Some(voice) = &self.voice {
            session["audio"]["output"]["voice"] = json!(voice);
        }
        session
    }
}

pub(super) fn decode(
    raw: &Value,
    max_audio_bytes: usize,
    format: LiveFormat,
    sideband: bool,
) -> Result<Vec<WireEvent>, String> {
    let kind = required_string(raw, "type")?;
    let event = match kind.as_str() {
        "session.started" => Some(WireEvent::Ready {
            session_id: raw
                .get("session")
                .ok_or_else(|| "GPT-Live startup has no session".to_string())
                .and_then(|s| required_string(s, "id"))?,
            configuration: raw.get("session").cloned().unwrap_or(Value::Null),
        }),
        "session.updated" => Some(WireEvent::ConfigurationUpdated),
        "session.input_audio.muted" | "session.input_audio.unmuted" => {
            Some(WireEvent::InputMuted {
                client_event_id: required_string(raw, "client_event_id")?,
                muted: kind == "session.input_audio.muted",
            })
        }
        "session.output_audio.delta" if !sideband => Some(WireEvent::Audio {
            response_id: None,
            item_id: None,
            bytes: decode_audio(raw, "delta", max_audio_bytes, format.sample_bytes())?,
            // Primary WS audio has no server timestamps; synthesized media
            // cursors belong to the adapter, and never imply heard words.
            start_ms: None,
            end_ms: None,
        }),
        // Reflected native media is intentionally discarded. Native tracks
        // carry playback and capture; forwarding this audio would duplicate it.
        "session.output_audio.delta" | "session.input_audio.append" => None,
        "session.input_transcript.delta" | "session.output_transcript.delta" => {
            let (start_ms, end_ms) = timeline_pair(raw).ok_or_else(|| {
                "GPT-Live transcript has no valid bounded timeline interval".to_string()
            })?;
            Some(WireEvent::Transcript {
                response_id: None,
                user: kind == "session.input_transcript.delta",
                fragment_id: string(raw, "event_id")
                    .unwrap_or_else(|| format!("{kind}:{start_ms}:{end_ms}")),
                text: string(raw, "delta").unwrap_or_default(),
                // This event fixes a fragment, not a complete user turn.
                complete: true,
                append: false,
                start_ms: Some(start_ms),
                end_ms: Some(end_ms),
            })
        }
        "session.delegation.created" => {
            let delegation = raw
                .get("delegation")
                .ok_or_else(|| "GPT-Live delegation has no metadata".to_string())?;
            if string(delegation, "target").as_deref() != Some("client") {
                return Err("GPT-Live adapter requires client delegation".into());
            }
            Some(WireEvent::Delegation {
                id: required_string(delegation, "id")?,
                offset_ms: raw
                    .get("offset_ms")
                    .and_then(|value| timeline_ms(value, false))
                    .ok_or_else(|| "GPT-Live delegation has no offset_ms".to_string())?,
            })
        }
        "session.instructions.appended"
        | "session.thinking.appended"
        | "session.commentary.appended" => Some(WireEvent::ContextAccepted {
            client_event_id: string(raw, "client_event_id"),
            start_ms: timeline_pair(raw).map(|(start, _)| start),
            end_ms: timeline_pair(raw).map(|(_, end)| end),
        }),
        "session.usage.updated" => Some(WireEvent::Usage {
            seconds: usage(raw)?,
        }),
        "session.closed" => Some(WireEvent::Closed {
            reason: required_string(raw, "reason")?,
        }),
        "error" => Some(WireEvent::Error {
            code: raw.get("error").and_then(|error| string(error, "code")),
            message: raw
                .get("error")
                .and_then(|error| string(error, "message"))
                .unwrap_or_else(|| "GPT-Live session error".into()),
        }),
        _ => None,
    };
    Ok(event.into_iter().collect())
}

fn usage(raw: &Value) -> Result<f64, String> {
    let seconds = raw
        .get("usage")
        .and_then(|usage| usage.get("seconds"))
        .and_then(Value::as_f64)
        .ok_or_else(|| "GPT-Live usage has no duration".to_string())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("GPT-Live usage duration is invalid".into());
    }
    Ok(seconds)
}

// The official timeline fields are numbers, not necessarily integer millis.
// Expand transcript/checkpoint intervals conservatively and round delegation
// offsets down; rounding cannot admit audio beyond a trigger or hide a target
// boundary. Invalid/overflowing clocks never establish a work checkpoint.
fn timeline_ms(value: &Value, ceil: bool) -> Option<u64> {
    let number = value.as_f64()?;
    if !number.is_finite() || number < 0.0 || number > (u64::MAX / 1000) as f64 {
        return None;
    }
    Some(if ceil { number.ceil() } else { number.floor() } as u64)
}
fn timeline_pair(value: &Value) -> Option<(u64, u64)> {
    let start = value.get("start_ms")?;
    let end = value.get("end_ms")?;
    if end.as_f64()? < start.as_f64()? {
        return None;
    }
    Some((timeline_ms(start, false)?, timeline_ms(end, true)?))
}

pub(super) fn context_append(
    content: &str,
    delegation_id: Option<&str>,
    speak: bool,
    event_id: &str,
) -> Value {
    json!({"type":if speak{"session.commentary.append"}else{"session.thinking.append"},
        "event_id":event_id,"delegation_id":delegation_id,"content":content})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_control_ack_requires_the_real_client_event_identity() {
        assert_eq!(
            decode(
                &json!({"type":"session.input_audio.muted","client_event_id":"input-control-1"}),
                48000,
                LiveFormat::Pcm24k,
                false
            )
            .unwrap(),
            vec![WireEvent::InputMuted {
                client_event_id: "input-control-1".into(),
                muted: true
            }]
        );
        assert_eq!(
            decode(
                &json!({"type":"session.input_audio.unmuted","client_event_id":"input-control-2"}),
                48000,
                LiveFormat::Pcm24k,
                false
            )
            .unwrap(),
            vec![WireEvent::InputMuted {
                client_event_id: "input-control-2".into(),
                muted: false
            }]
        );
        assert!(
            decode(
                &json!({"type":"session.input_audio.muted","event_id":"server-event"}),
                48000,
                LiveFormat::Pcm24k,
                false
            )
            .is_err()
        );
    }
    #[test]
    fn context_ack_and_input_use_same_bounded_timeline_without_local_clock_guess() {
        assert_eq!(
            timeline_pair(&json!({"start_ms":1200.25,"end_ms":1400.5})),
            Some((1200, 1401))
        );
        assert!(timeline_pair(&json!({"start_ms":1000.9,"end_ms":1000.1})).is_none());
        assert!(timeline_pair(&json!({"start_ms":u64::MAX,"end_ms":u64::MAX})).is_none());
        assert!(
            matches!(decode(&json!({"type":"session.thinking.appended","client_event_id":"verified-fact","start_ms":1200,"end_ms":1400}),48000,LiveFormat::Pcm24k,false).unwrap().as_slice(),
            [WireEvent::ContextAccepted{client_event_id:Some(id),start_ms:Some(1200),end_ms:Some(1400)}] if id=="verified-fact")
        );
    }
    #[test]
    fn metadata_delegation_never_fabricates_tool_arguments() {
        let events=decode(&json!({"type":"session.delegation.created","offset_ms":1000,"delegation":{"id":"opaque","type":"delegation","target":"client"}}),48000,LiveFormat::Pcm24k,false).unwrap();
        assert_eq!(
            events,
            vec![WireEvent::Delegation {
                id: "opaque".into(),
                offset_ms: 1000
            }]
        );
    }
    #[test]
    fn native_attachment_has_no_pcm_format_and_reflected_audio_is_not_played() {
        let config = LiveConfig::parse(&json!({})).unwrap();
        assert!(
            config
                .session("catalog-model", "", json!([]), true)
                .get("audio")
                .and_then(|audio| audio.get("format"))
                .is_none()
        );
        assert!(
            decode(
                &json!({"type":"session.output_audio.delta","delta":"AAAA"}),
                48000,
                LiveFormat::Pcm24k,
                true
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn transcript_preserves_spaces_and_independent_timeline() {
        let events=decode(&json!({"type":"session.input_transcript.delta","event_id":"e","delta":" Thursday, not Friday","start_ms":1000,"end_ms":1400}),48000,LiveFormat::Pcm24k,false).unwrap();
        assert!(
            matches!(events.as_slice(),[WireEvent::Transcript{text,start_ms:Some(1000),end_ms:Some(1400),..}] if text==" Thursday, not Friday")
        );
    }
    #[test]
    fn relay_codec_supports_different_rates_and_rejects_partial_pcm_sample() {
        assert_eq!(LiveFormat::Pcm16k.rate(), 16000);
        assert!(
            decode(
                &json!({"type":"session.output_audio.delta","delta":"AA=="}),
                48000,
                LiveFormat::Pcm16k,
                false
            )
            .is_err()
        );
        assert!(
            decode(
                &json!({"type":"session.output_audio.delta","delta":"AA=="}),
                48000,
                LiveFormat::Pcmu8k,
                false
            )
            .is_ok()
        );
    }
}
