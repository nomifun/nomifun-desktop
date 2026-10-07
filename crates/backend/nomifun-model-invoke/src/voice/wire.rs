//! Private protocol decoding facts. None of these types cross the voice port.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum WireEvent {
    Ready {
        session_id: String,
        configuration: Value,
    },
    ConfigurationUpdated,
    InputMuted {
        client_event_id: String,
        muted: bool,
    },
    InputCleared {
        client_event_id: Option<String>,
    },
    MessageAccepted {
        item: Value,
    },
    Audio {
        response_id: Option<String>,
        item_id: Option<String>,
        bytes: Vec<u8>,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
    },
    AudioDone {
        response_id: Option<String>,
        item_id: Option<String>,
    },
    Transcript {
        user: bool,
        response_id: Option<String>,
        fragment_id: String,
        text: String,
        complete: bool,
        append: bool,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
    },
    Speech {
        active: bool,
    },
    OutputStarted {
        response_id: String,
    },
    OutputInterrupted {
        response_id: Option<String>,
    },
    ToolCall {
        call_id: String,
        response_id: Option<String>,
        name: String,
        arguments: Value,
    },
    Delegation {
        id: String,
        offset_ms: u64,
    },
    ContextAccepted {
        client_event_id: Option<String>,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
    },
    Usage {
        seconds: f64,
    },
    Closed {
        reason: String,
    },
    Error {
        code: Option<String>,
        message: String,
    },
}

pub(super) fn string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

pub(super) fn required_string(value: &Value, field: &str) -> Result<String, String> {
    string(value, field)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("voice protocol event requires a non-empty {field}"))
}

pub(super) fn decode_audio(
    value: &Value,
    field: &str,
    max_bytes: usize,
    sample_bytes: usize,
) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let encoded = required_string(value, field)?;
    // Bound the allocation before base64 decoding as well as afterwards.
    if encoded.len() > max_bytes.div_ceil(3).saturating_mul(4) {
        return Err("voice protocol audio exceeds negotiated frame limit".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "voice protocol audio has invalid base64".to_string())?;
    if decoded.len() > max_bytes || !decoded.len().is_multiple_of(sample_bytes) {
        return Err("voice protocol audio violates negotiated sample/frame limits".into());
    }
    Ok(decoded)
}
