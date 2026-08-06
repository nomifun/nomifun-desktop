//! Opus wrappers pinned to the two shapes this gateway needs.
//!
//! Uplink is fixed by firmware: 16 kHz mono 60 ms. Downlink is what we declared
//! in the server hello: 24 kHz mono 60 ms. Nothing else is supported on purpose
//! — a mismatched frame size makes the device's decoder fail outright.
//!
//! The binding is `opusic-sys`, which vendors libopus and links it **statically**
//! via CMake. That matters for packaging: no machine needs a preinstalled
//! libopus, and no shared object has to be shipped beside the binary. The safe
//! wrappers below are the whole FFI surface — six libopus entry points — so a
//! future swap to a higher-level crate touches only this file.

use std::ffi::{CStr, c_int};

use opusic_sys as ffi;

use crate::protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS, UPLINK_SAMPLE_RATE};

/// Samples in one 60 ms uplink frame (16 kHz).
pub const UPLINK_FRAME_SAMPLES: usize = (UPLINK_SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;
/// Samples in one 60 ms downlink frame (24 kHz).
pub const DOWNLINK_FRAME_SAMPLES: usize =
    (DOWNLINK_SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;

/// Largest Opus packet we will ever produce or accept, per RFC 6716.
const MAX_PACKET_BYTES: usize = 1275;
/// Mono, in both directions, always.
const CHANNELS: c_int = 1;

/// libopus only accepts these rates; reject anything else before it reaches FFI.
fn check_rate(hz: u32) -> anyhow::Result<i32> {
    match hz {
        8_000 | 12_000 | 16_000 | 24_000 | 48_000 => Ok(hz as i32),
        other => anyhow::bail!("unsupported opus sample rate {other}"),
    }
}

/// Turn a negative libopus return code into an error carrying its own message.
fn opus_error(context: &str, code: c_int) -> anyhow::Error {
    // SAFETY: `opus_strerror` returns a pointer to a static NUL-terminated
    // string for every input, including unknown codes.
    let message = unsafe { CStr::from_ptr(ffi::opus_strerror(code)) };
    anyhow::anyhow!(
        "{context} failed: {} (code {code})",
        message.to_string_lossy()
    )
}

/// Decodes uplink packets from the device.
pub struct OpusStreamDecoder {
    inner: *mut ffi::OpusDecoder,
    frame_samples: usize,
}

// SAFETY: an `OpusDecoder` is a plain heap allocation with no interior thread
// affinity, and every method here takes `&mut self`, so libopus never sees
// concurrent use of the same state.
unsafe impl Send for OpusStreamDecoder {}

impl OpusStreamDecoder {
    /// 16 kHz mono — the only uplink shape the firmware produces.
    pub fn new_uplink() -> anyhow::Result<Self> {
        Self::new(UPLINK_SAMPLE_RATE, UPLINK_FRAME_SAMPLES)
    }

    fn new(sample_rate: u32, frame_samples: usize) -> anyhow::Result<Self> {
        let rate = check_rate(sample_rate)?;
        let mut error: c_int = 0;
        // SAFETY: `rate`/`CHANNELS` are validated libopus arguments and `error`
        // is a live local; the returned pointer is checked before use.
        let inner = unsafe { ffi::opus_decoder_create(rate, CHANNELS, &mut error) };
        if error != ffi::OPUS_OK || inner.is_null() {
            return Err(opus_error("opus_decoder_create", error));
        }
        Ok(Self {
            inner,
            frame_samples,
        })
    }

    /// Decode one packet into PCM. The buffer is sized for a full frame and
    /// truncated to what Opus actually produced.
    pub fn decode(&mut self, packet: &[u8]) -> anyhow::Result<Vec<i16>> {
        let mut pcm = vec![0i16; self.frame_samples];
        // SAFETY: `packet` and `pcm` are live slices whose lengths are passed
        // alongside their pointers, and `self.inner` is a non-null decoder owned
        // by `self`.
        let produced = unsafe {
            ffi::opus_decode(
                self.inner,
                packet.as_ptr(),
                packet.len() as i32,
                pcm.as_mut_ptr(),
                self.frame_samples as c_int,
                0,
            )
        };
        if produced < 0 {
            return Err(opus_error("opus_decode", produced));
        }
        pcm.truncate(produced as usize);
        Ok(pcm)
    }
}

impl Drop for OpusStreamDecoder {
    fn drop(&mut self) {
        // SAFETY: `self.inner` came from `opus_decoder_create`, is non-null, and
        // is destroyed exactly once.
        unsafe { ffi::opus_decoder_destroy(self.inner) };
    }
}

/// Encodes PCM into 60 ms packets.
pub struct OpusStreamEncoder {
    inner: *mut ffi::OpusEncoder,
    frame_samples: usize,
}

// SAFETY: see [`OpusStreamDecoder`] — same reasoning for the encoder state.
unsafe impl Send for OpusStreamEncoder {}

impl OpusStreamEncoder {
    /// 24 kHz mono — matches the `audio_params` in our server hello.
    pub fn new_downlink() -> anyhow::Result<Self> {
        Self::new(DOWNLINK_SAMPLE_RATE, DOWNLINK_FRAME_SAMPLES)
    }

    /// 16 kHz mono. Only used by tests that mimic the device's uplink.
    pub fn new_uplink_for_test() -> anyhow::Result<Self> {
        Self::new(UPLINK_SAMPLE_RATE, UPLINK_FRAME_SAMPLES)
    }

    fn new(sample_rate: u32, frame_samples: usize) -> anyhow::Result<Self> {
        let rate = check_rate(sample_rate)?;
        let mut error: c_int = 0;
        // SAFETY: as in `OpusStreamDecoder::new`; `OPUS_APPLICATION_VOIP` is the
        // speech-tuned mode libopus documents for conversational audio.
        let inner = unsafe {
            ffi::opus_encoder_create(rate, CHANNELS, ffi::OPUS_APPLICATION_VOIP, &mut error)
        };
        if error != ffi::OPUS_OK || inner.is_null() {
            return Err(opus_error("opus_encoder_create", error));
        }
        Ok(Self {
            inner,
            frame_samples,
        })
    }

    /// Split `pcm` into whole frames and encode each. A trailing partial frame
    /// is zero-padded rather than dropped, otherwise the last syllable of every
    /// sentence disappears.
    pub fn encode_frames(&mut self, pcm: &[i16]) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        for chunk in pcm.chunks(self.frame_samples) {
            let mut frame = chunk.to_vec();
            frame.resize(self.frame_samples, 0);
            let mut packet = vec![0u8; MAX_PACKET_BYTES];
            // SAFETY: `frame` holds exactly `frame_samples` mono samples and
            // `packet` is `MAX_PACKET_BYTES` long; both lengths travel with
            // their pointers.
            let written = unsafe {
                ffi::opus_encode(
                    self.inner,
                    frame.as_ptr(),
                    self.frame_samples as c_int,
                    packet.as_mut_ptr(),
                    packet.len() as i32,
                )
            };
            if written < 0 {
                return Err(opus_error("opus_encode", written));
            }
            packet.truncate(written as usize);
            out.push(packet);
        }
        Ok(out)
    }
}

impl Drop for OpusStreamEncoder {
    fn drop(&mut self) {
        // SAFETY: `self.inner` came from `opus_encoder_create`, is non-null, and
        // is destroyed exactly once.
        unsafe { ffi::opus_encoder_destroy(self.inner) };
    }
}
