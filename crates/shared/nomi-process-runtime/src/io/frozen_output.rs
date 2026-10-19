use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use super::{FrozenOutput, IncrementalDecoder, OutputState, StreamDecoders, UTF8_LABEL};

const FORMAT_VERSION: u32 = 1;
// The decoder implementation is part of the receipt. Replaying a Windows
// receipt with a UTF-8-only decoder must fail instead of changing its text.
const DECODER_SCHEME: &str = if cfg!(windows) {
    "windows_acp_v1"
} else {
    "utf8_v1"
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrozenOutputError {
    #[error("unsupported frozen process output version {0}")]
    UnsupportedVersion(u32),
    #[error("unsupported frozen process output decoder {0}")]
    UnsupportedDecoder(String),
    #[error("invalid frozen process output: {0}")]
    InvalidState(&'static str),
}

#[derive(Serialize)]
struct BorrowedReceipt<'a> {
    version: u32,
    decoder_scheme: &'static str,
    state: &'a OutputState,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedReceipt {
    version: u32,
    decoder_scheme: String,
    state: OutputState,
}

impl Serialize for FrozenOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        BorrowedReceipt {
            version: FORMAT_VERSION,
            decoder_scheme: DECODER_SCHEME,
            state: &self.state,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FrozenOutput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let receipt = OwnedReceipt::deserialize(deserializer)?;
        if receipt.version != FORMAT_VERSION {
            return Err(D::Error::custom(FrozenOutputError::UnsupportedVersion(
                receipt.version,
            )));
        }
        if receipt.decoder_scheme != DECODER_SCHEME {
            return Err(D::Error::custom(FrozenOutputError::UnsupportedDecoder(
                receipt.decoder_scheme,
            )));
        }
        let frozen = Self {
            state: Arc::new(receipt.state),
        };
        frozen.validate().map_err(D::Error::custom)?;
        Ok(frozen)
    }
}

impl std::fmt::Debug for FrozenOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenOutput")
            .field("next_offset", &self.state.next_offset)
            .field("retained_bytes", &self.state.retained)
            .field("dropped_bytes", &self.state.dropped_bytes)
            .finish_non_exhaustive()
    }
}

impl FrozenOutput {
    /// Validate the raw cursor/chunk/decoder/EOF invariants before read-only
    /// replay. Deserialization calls this automatically. There is deliberately
    /// no conversion back into an OutputBuffer or a native process owner.
    pub fn validate(&self) -> Result<(), FrozenOutputError> {
        validate_state(&self.state)
    }
}

fn invalid(reason: &'static str) -> FrozenOutputError {
    FrozenOutputError::InvalidState(reason)
}

fn validate_state(state: &OutputState) -> Result<(), FrozenOutputError> {
    if !state.finalized {
        return Err(invalid("terminal EOF has not been frozen"));
    }
    if state.base_offset != state.dropped_bytes {
        return Err(invalid("base offset differs from dropped raw bytes"));
    }
    let retained = u64::try_from(state.retained).map_err(|_| invalid("retained byte count is too large"))?;
    if state.next_offset.checked_sub(state.base_offset) != Some(retained) {
        return Err(invalid("retained byte count does not cover the raw cursor range"));
    }
    // EOF-only chunks use next_seq + the stream ordinal in the original
    // snapshot implementation. Reject a corrupt value before replay can panic.
    if state.next_seq.checked_add(2).is_none() {
        return Err(invalid("terminal chunk sequence would overflow"));
    }
    let mut offset = state.base_offset;
    let mut previous_seq = None;
    for chunk in &state.chunks {
        if chunk.bytes.is_empty() || chunk.start != offset {
            return Err(invalid("retained raw chunks are empty or discontinuous"));
        }
        if chunk.seq >= state.next_seq
            || previous_seq.is_some_and(|previous| previous >= chunk.seq)
        {
            return Err(invalid("retained chunk sequence is not strictly increasing"));
        }
        offset = offset
            .checked_add(u64::try_from(chunk.bytes.len()).map_err(|_| invalid("raw chunk is too large"))?)
            .ok_or_else(|| invalid("raw chunk cursor overflows"))?;
        previous_seq = Some(chunk.seq);
    }
    if offset != state.next_offset {
        return Err(invalid("retained chunks do not reach the original raw cursor"));
    }
    validate_decoders(&state.base_decoders, false)?;
    validate_decoders(&state.decoders, true)?;
    Ok(())
}

fn validate_decoders(decoders: &StreamDecoders, finalized: bool) -> Result<(), FrozenOutputError> {
    for decoder in [&decoders.stdout, &decoders.stderr, &decoders.pty] {
        validate_decoder(decoder, finalized)?;
    }
    Ok(())
}

fn validate_decoder(decoder: &IncrementalDecoder, finalized: bool) -> Result<(), FrozenOutputError> {
    if decoder.utf8_pending.len() > 3
        || (!decoder.utf8_pending.is_empty()
            && !matches!(std::str::from_utf8(&decoder.utf8_pending), Err(error)
                if error.valid_up_to() == 0 && error.error_len().is_none()))
    {
        return Err(invalid("invalid UTF-8 decoder carry"));
    }
    if finalized && !decoder.utf8_pending.is_empty() {
        return Err(invalid("terminal decoder retains unfinished UTF-8 bytes"));
    }
    if !decoder.observed_bytes
        && (decoder.decode_errors != 0
            || decoder.saw_non_ascii_utf8
            || !decoder.utf8_pending.is_empty())
    {
        return Err(invalid("unobserved stream contains decoder history"));
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::IsValidCodePage;
        // SAFETY: this only checks an identifier captured from the original
        // decoder; it creates no native handles or process authority.
        if decoder.active_code_page == 0 || unsafe { IsValidCodePage(decoder.active_code_page) } == 0 {
            return Err(invalid("original Windows code page is unavailable"));
        }
        if let Some(windows) = &decoder.windows {
            if !decoder.observed_bytes
                || !decoder.utf8_pending.is_empty()
                || windows.code_page != decoder.active_code_page
                || windows.code_page == 65001
                || windows.pending.len() > 1
                || (!windows.pending.is_empty()
                    && super::trailing_dbcs_lead(windows.code_page, &windows.pending) != Some(0))
                || (finalized && !windows.pending.is_empty())
            {
                return Err(invalid("invalid Windows decoder state or carry"));
            }
            let expected = if decoder.saw_non_ascii_utf8 {
                "mixed".to_owned()
            } else {
                windows.label()
            };
            if decoder.source_encoding != expected {
                return Err(invalid("Windows decoder encoding metadata is inconsistent"));
            }
            return Ok(());
        }
    }
    if decoder.source_encoding != UTF8_LABEL {
        return Err(invalid("UTF-8 decoder encoding metadata is inconsistent"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
