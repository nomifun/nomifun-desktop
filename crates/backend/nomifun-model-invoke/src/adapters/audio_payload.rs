//! Minimal audio-result boundary shared by synchronous speech/music codecs.
//! Audio need not have a container signature (PCM is raw bytes), but an empty
//! body or a structured error/document must never become a playable artifact.

use crate::error::InvokeError;

pub(crate) fn validate_audio_payload(
    bytes: &[u8], content_type: Option<&str>, context: &str,
) -> Result<(), InvokeError> {
    if bytes.is_empty() {
        return Err(InvokeError::parse(format!("{context} returned empty audio")));
    }
    let mime = content_type.map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase());
    if mime.as_deref().is_some_and(|mime|
        mime.starts_with("text/") || mime=="application/json" || mime.ends_with("+json")
            || mime=="application/xml" || mime.ends_with("+xml")) {
        return Err(InvokeError::parse(format!("{context} returned a structured/text response instead of audio")));
    }
    if nomifun_net::api_response::looks_like_markup(bytes) {
        return Err(InvokeError::parse(format!("{context} returned markup instead of audio")));
    }
    let without_bom = bytes.strip_prefix(&[0xEF,0xBB,0xBF]).unwrap_or(bytes);
    let trimmed = without_bom.trim_ascii_start();
    // Parse only JSON containers, without allocating a second response tree.
    // A digit, quote or brace byte in arbitrary PCM is not enough to reject it.
    if matches!(trimmed.first(),Some(b'{' | b'['))
        && serde_json::from_slice::<serde::de::IgnoredAny>(trimmed).is_ok() {
        return Err(InvokeError::parse(format!("{context} returned JSON instead of audio")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_disguised_documents_without_requiring_an_audio_container() {
        for (bytes,mime) in [
            (&b""[..],Some("audio/mpeg")),
            (&b"{\"error\":\"upstream failed\"}"[..],Some("audio/mpeg")),
            (&b"\xef\xbb\xbf  {}"[..],None),
            (&b"[]"[..],Some("application/octet-stream")),
            (&b"<!doctype html><html>gateway</html>"[..],Some("audio/wav")),
            (&b"opaque"[..],Some("application/problem+json; charset=utf-8")),
            (&b"data: {}\n\n"[..],Some("text/event-stream")),
        ] {assert!(validate_audio_payload(bytes,mime,"fixture").is_err());}
        for bytes in [&b"ID3audio"[..],&b"RIFFaudio"[..],&b"\0\x01\0\x02"[..],&b"{\0\xff"[..],&b"1234"[..]] {
            assert!(validate_audio_payload(bytes,Some("audio/pcm"),"fixture").is_ok());
        }
    }
}
