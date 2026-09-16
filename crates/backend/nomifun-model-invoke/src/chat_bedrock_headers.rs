//! AWS event-stream header decoding after frame bounds and both CRCs pass.
//! Preserve machine-readable failures, never remote diagnostic strings. This
//! layer does not retry, choose routes or infer context limits from messages.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::SingleAttemptFrame;
use crate::error::InvokeError;

fn invalid() -> InvokeError {
    InvokeError::parse("invalid Bedrock event-stream headers or exception envelope")
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> Result<&'a [u8], InvokeError> {
    if input.len() < count {
        return Err(invalid());
    }
    let (value, rest) = input.split_at(count);
    *input = rest;
    Ok(value)
}

/// None denotes a model event. Some denotes a terminal protocol exception,
/// projected only as an explicit machine code for the Broker to classify.
pub(super) fn exception_frame(
    mut headers: &[u8],
    payload: &[u8],
) -> Result<Option<SingleAttemptFrame>, InvokeError> {
    // Existing headerless gateway framing remains supported, but cannot claim
    // any exception identity based on its human-readable payload.
    if headers.is_empty() {
        return Ok(None);
    }
    if headers.len() > 128 * 1024 {
        return Err(invalid());
    }
    let mut values = BTreeMap::new();
    while !headers.is_empty() {
        if values.len() >= 128 {
            return Err(invalid());
        }
        let name_len = usize::from(take(&mut headers, 1)?[0]);
        if name_len == 0 {
            return Err(invalid());
        }
        let name = std::str::from_utf8(take(&mut headers, name_len)?).map_err(|_| invalid())?;
        let tag = take(&mut headers, 1)?[0];
        // AWS event-stream types: bool true/false, byte, short, int, long,
        // byte array, string, timestamp, UUID. Length prefixes are big endian.
        let value = match tag {
            0 | 1 => None,
            2 | 3 | 4 | 5 | 8 | 9 => {
                let len = match tag {
                    2 => 1,
                    3 => 2,
                    4 => 4,
                    9 => 16,
                    _ => 8,
                };
                take(&mut headers, len)?;
                None
            }
            6 | 7 => {
                let size = take(&mut headers, 2)?;
                let size = usize::from(u16::from_be_bytes([size[0], size[1]]));
                let bytes = take(&mut headers, size)?;
                if tag == 7 {
                    Some(std::str::from_utf8(bytes).map_err(|_| invalid())?)
                } else {
                    None
                }
            }
            _ => return Err(invalid()),
        };
        if name.starts_with(':') && value.is_none() {
            return Err(invalid());
        }
        if values.insert(name, value).is_some() {
            return Err(invalid());
        }
    }
    let string = |key: &str| values.get(key).copied().flatten();
    if string(":content-type").is_some_and(|value| value != "application/json") {
        return Err(invalid());
    }
    let code = match string(":message-type") {
        Some("event") => {
            if string(":event-type") != Some("chunk")
                || values.contains_key(":exception-type")
                || values.contains_key(":error-code")
                || values.contains_key(":error-message")
            {
                return Err(invalid());
            }
            return Ok(None);
        }
        Some("exception") => {
            if values.contains_key(":event-type")
                || values.contains_key(":error-code")
                || values.contains_key(":error-message")
            {
                return Err(invalid());
            }
            string(":exception-type").ok_or_else(invalid)?
        }
        Some("error") => {
            if values.contains_key(":event-type") || values.contains_key(":exception-type") {
                return Err(invalid());
            }
            string(":error-code").ok_or_else(invalid)?
        }
        _ => return Err(invalid()),
    };
    if code.is_empty()
        || code.len() > 128
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(invalid());
    }
    if !payload.is_empty() {
        let body: Value = serde_json::from_slice(payload).map_err(|_| invalid())?;
        let body = body.as_object().ok_or_else(invalid)?;
        // Exception bodies are diagnostics only. Do not discard a model chunk,
        // tool output or usage while granting retry based on an exception header.
        for (key, value) in body {
            let valid = match key.as_str() {
                "message" | "Message" | "originalMessage" => value.is_string() || value.is_null(),
                "originalStatusCode" => value
                    .as_u64()
                    .is_some_and(|code| (100..=599).contains(&code)),
                _ => false,
            };
            if !valid {
                return Err(invalid());
            }
        }
    }
    Ok(Some(SingleAttemptFrame {
        event: "bedrock.exception".to_owned(),
        data: json!({"error": {"code": code}}),
    }))
}
