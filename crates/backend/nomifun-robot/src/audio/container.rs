//! Container decode for TTS audio that did not come back as raw PCM.
//!
//! Providers differ: OpenAI-compatible endpoints honour `format: "pcm"`, others
//! return mp3 or an Ogg container regardless. symphonia probes the bytes, so the
//! mime hint is advisory only.

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::AudioBuffer;

/// Decode any container symphonia recognises into mono PCM.
/// Multi-channel input is downmixed by averaging.
pub fn decode_container(bytes: &[u8], mime_hint: Option<&str>) -> anyhow::Result<AudioBuffer> {
    let source = std::io::Cursor::new(bytes.to_vec());
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(mime) = mime_hint {
        hint.mime_type(mime);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow::anyhow!("audio has no default track"))?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        anyhow::bail!("default track carries no audio codec parameters");
    };
    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let mut pcm: Vec<i16> = Vec::new();
    let mut interleaved: Vec<i16> = Vec::new();
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let (rate, channels) = {
            let spec = decoded.spec();
            (spec.rate(), spec.channels().count().max(1))
        };
        if sample_rate == 0 {
            sample_rate = rate;
        }
        decoded.copy_to_vec_interleaved(&mut interleaved);
        if channels == 1 {
            pcm.extend_from_slice(&interleaved);
        } else {
            pcm.extend(interleaved.chunks(channels).map(|frame| {
                (frame.iter().map(|s| *s as i32).sum::<i32>() / channels as i32) as i16
            }));
        }
    }
    if pcm.is_empty() || sample_rate == 0 {
        anyhow::bail!("decoded no audio samples");
    }
    Ok(AudioBuffer { pcm, sample_rate })
}
