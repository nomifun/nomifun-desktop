//! Validation at the provider-output trust boundary.
//!
//! Provider responses are untrusted: a successful HTTP status can still carry
//! an empty body, an HTML/JSON error page, corrupt image bytes, or a MIME type
//! that does not match the requested capability. Nothing may reach an
//! [`crate::AssetSink`] until this module has established that it is a real,
//! usable artifact.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, Limits};

use crate::{CreationError, MediaCapability};

const MAX_IMAGE_EDGE: u32 = 32_768;
const MAX_IMAGE_DECODE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OGG_PACKET_BYTES: usize = 16 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> CreationError {
    CreationError::new("invalid_artifact", message)
}

/// Normalize a possibly parameterized MIME value. Empty values are treated as
/// absent, and the common non-standard `image/jpg` alias is canonicalized.
pub(crate) fn normalize_mime(value: &str) -> Option<String> {
    let mime = value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else {
        Some(match mime.as_str() {
            "image/jpg" | "image/pjpeg" => "image/jpeg".to_string(),
            "image/x-png" => "image/png".to_string(),
            "audio/x-wav" => "audio/wav".to_string(),
            "audio/mp3" => "audio/mpeg".to_string(),
            "video/x-m4v" => "video/mp4".to_string(),
            _ => mime,
        })
    }
}

fn is_generic_binary_mime(mime: &str) -> bool {
    matches!(mime, "application/octet-stream" | "binary/octet-stream")
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let prefix = prefix.trim_start_matches('\u{feff}').trim_start().to_ascii_lowercase();
    prefix.starts_with("<!doctype html")
        || prefix.starts_with("<html")
        || prefix.starts_with("<head")
        || prefix.starts_with("<body")
}

fn looks_like_json_document(bytes: &[u8]) -> bool {
    let first = bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace());
    matches!(first, Some(b'{') | Some(b'[')) && serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
}

fn reject_binary_error_document(bytes: &[u8], declared_mime: Option<&str>) -> Result<(), CreationError> {
    if bytes.is_empty() {
        return Err(invalid("provider produced an empty artifact"));
    }
    if declared_mime.is_some_and(|mime| matches!(mime, "text/html" | "application/xhtml+xml"))
        || looks_like_html(bytes)
    {
        return Err(invalid("provider returned an HTML document instead of an artifact"));
    }
    if declared_mime == Some("application/json") || looks_like_json_document(bytes) {
        return Err(invalid("provider returned a JSON document instead of a binary artifact"));
    }
    Ok(())
}

fn image_mime(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn validate_image(bytes: &[u8], declared_mime: Option<&str>) -> Result<String, CreationError> {
    reject_binary_error_document(bytes, declared_mime)?;
    let format = image::guess_format(bytes).map_err(|_| invalid("provider returned unrecognized image bytes"))?;
    let actual_mime = image_mime(format)
        .ok_or_else(|| invalid(format!("provider returned an unsupported image format: {format:?}")))?;

    if let Some(declared) = declared_mime.filter(|mime| !is_generic_binary_mime(mime))
        && declared != actual_mime
    {
        return Err(invalid(format!(
            "artifact MIME mismatch: declared '{declared}', detected '{actual_mime}'"
        )));
    }

    // Signature checks alone are insufficient: truncated/corrupt files can
    // still have a valid magic number. Decode under explicit allocation and
    // dimension limits before the task is allowed to succeed.
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_BYTES);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| invalid("provider returned a corrupt or unsafe image payload"))?;

    Ok(actual_mime.to_string())
}

fn validate_text(bytes: &[u8], declared_mime: Option<&str>) -> Result<String, CreationError> {
    if let Some(declared) = declared_mime.filter(|mime| !is_generic_binary_mime(mime))
        && declared != "text/plain"
    {
        return Err(invalid(format!(
            "artifact MIME mismatch: expected 'text/plain', received '{declared}'"
        )));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("text artifact is not valid UTF-8"))?;
    if text.trim().is_empty() {
        return Err(invalid("provider produced an empty text artifact"));
    }
    Ok("text/plain".to_string())
}

fn read_be_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn read_le_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

const MAX_CONTAINER_ELEMENTS: usize = 10_000;

#[derive(Clone, Copy)]
struct BmffBox<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}

#[derive(Clone, Copy)]
struct BmffInfo {
    brand: [u8; 4],
    has_audio: bool,
    has_video: bool,
}

fn bmff_boxes(bytes: &[u8]) -> Option<Vec<BmffBox<'_>>> {
    let mut offset = 0usize;
    let mut boxes = Vec::new();
    while offset < bytes.len() {
        if boxes.len() >= MAX_CONTAINER_ELEMENTS {
            return None;
        }
        let header = bytes.get(offset..offset.checked_add(8)?)?;
        let short_size = read_be_u32(header)? as usize;
        let kind: [u8; 4] = header[4..8].try_into().ok()?;
        let (header_len, box_len) = match short_size {
            0 => (8usize, bytes.len().checked_sub(offset)?),
            1 => {
                let wide = u64::from_be_bytes(bytes.get(offset + 8..offset + 16)?.try_into().ok()?);
                (16usize, usize::try_from(wide).ok()?)
            }
            size => (8usize, size),
        };
        let end = offset.checked_add(box_len)?;
        if box_len < header_len || end > bytes.len() || (short_size == 0 && end != bytes.len()) {
            return None;
        }
        boxes.push(BmffBox {
            kind,
            payload: bytes.get(offset + header_len..end)?,
        });
        offset = end;
    }
    (offset == bytes.len()).then_some(boxes)
}

fn bmff_box<'a>(boxes: &'a [BmffBox<'a>], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes.iter().find(|item| &item.kind == kind).map(|item| item.payload)
}

fn full_box_entry_count(payload: &[u8], entry_width: usize) -> Option<usize> {
    let count = usize::try_from(read_be_u32(payload.get(4..)?)?).ok()?;
    let required = 8usize.checked_add(count.checked_mul(entry_width)?)?;
    (count > 0 && payload.len() >= required).then_some(count)
}

fn valid_sample_description(payload: &[u8]) -> bool {
    let Some(count) = full_box_entry_count(payload, 8) else {
        return false;
    };
    let Some(entries) = bmff_boxes(&payload[8..]) else {
        return false;
    };
    entries.len() >= count
        && entries.iter().take(count).all(|entry| {
            entry.kind.iter().any(|byte| *byte != 0)
                && !entry.payload.is_empty()
                && entry.kind.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' ')
        })
}

fn valid_sample_table(payload: &[u8], fragmented: bool) -> bool {
    let Some(boxes) = bmff_boxes(payload) else {
        return false;
    };
    if !bmff_box(&boxes, b"stsd").is_some_and(valid_sample_description) {
        return false;
    }
    if fragmented {
        return true;
    }
    let valid_stts = bmff_box(&boxes, b"stts").is_some_and(|value| {
        let Some(count) = full_box_entry_count(value, 8) else {
            return false;
        };
        (0..count).any(|index| {
            let start = 8 + index * 8;
            read_be_u32(&value[start..]).is_some_and(|samples| samples > 0)
        })
    });
    let valid_stsc = bmff_box(&boxes, b"stsc").is_some_and(|value| full_box_entry_count(value, 12).is_some());
    let valid_stsz = bmff_box(&boxes, b"stsz").is_some_and(|value| {
        let Some(sample_count) = read_be_u32(value.get(8..).unwrap_or_default())
            .and_then(|count| usize::try_from(count).ok())
        else {
            return false;
        };
        if sample_count == 0 || value.len() < 12 {
            return false;
        }
        let fixed_size = read_be_u32(value.get(4..).unwrap_or_default()).unwrap_or(0);
        fixed_size > 0 || value.len() >= 12usize.saturating_add(sample_count.saturating_mul(4))
    });
    let valid_chunk_offsets = bmff_box(&boxes, b"stco")
        .is_some_and(|value| full_box_entry_count(value, 4).is_some())
        || bmff_box(&boxes, b"co64").is_some_and(|value| full_box_entry_count(value, 8).is_some());
    valid_stts && valid_stsc && valid_stsz && valid_chunk_offsets
}

fn bmff_track_kind(payload: &[u8], fragmented: bool) -> Option<[u8; 4]> {
    let trak = bmff_boxes(payload)?;
    if !bmff_box(&trak, b"tkhd").is_some_and(|value| value.len() >= 8) {
        return None;
    }
    let mdia = bmff_boxes(bmff_box(&trak, b"mdia")?)?;
    if !bmff_box(&mdia, b"mdhd").is_some_and(|value| value.len() >= 8) {
        return None;
    }
    let handler = bmff_box(&mdia, b"hdlr")?;
    let handler_kind: [u8; 4] = handler.get(8..12)?.try_into().ok()?;
    if !matches!(&handler_kind, b"vide" | b"soun") {
        return None;
    }
    let minf = bmff_boxes(bmff_box(&mdia, b"minf")?)?;
    if !bmff_box(&minf, b"stbl").is_some_and(|value| valid_sample_table(value, fragmented)) {
        return None;
    }
    Some(handler_kind)
}

fn valid_movie_fragment(payload: &[u8]) -> bool {
    let Some(moof) = bmff_boxes(payload) else {
        return false;
    };
    if !bmff_box(&moof, b"mfhd").is_some_and(|value| value.len() >= 8) {
        return false;
    }
    moof.iter().filter(|item| &item.kind == b"traf").any(|traf| {
        let Some(children) = bmff_boxes(traf.payload) else {
            return false;
        };
        bmff_box(&children, b"tfhd").is_some_and(|value| value.len() >= 8)
            && children.iter().filter(|item| &item.kind == b"trun").any(|trun| {
                trun.payload.len() >= 8 && read_be_u32(&trun.payload[4..]).is_some_and(|count| count > 0)
            })
    })
}

fn iso_bmff_info(bytes: &[u8]) -> Option<BmffInfo> {
    let top = bmff_boxes(bytes)?;
    let ftyp = bmff_box(&top, b"ftyp")?;
    let brand: [u8; 4] = ftyp.get(..4)?.try_into().ok()?;
    if ftyp.len() < 8
        || !brand.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b' ')
        || brand.iter().all(|byte| *byte == 0)
    {
        return None;
    }
    let moov_payload = bmff_box(&top, b"moov")?;
    let moov = bmff_boxes(moov_payload)?;
    if !bmff_box(&moov, b"mvhd").is_some_and(|value| value.len() >= 8) {
        return None;
    }
    let has_mvex = bmff_box(&moov, b"mvex").is_some();
    let fragments = top.iter().filter(|item| &item.kind == b"moof").collect::<Vec<_>>();
    let fragmented = has_mvex || !fragments.is_empty();
    if fragmented && (!has_mvex || fragments.is_empty() || !fragments.iter().all(|item| valid_movie_fragment(item.payload))) {
        return None;
    }
    let mut has_audio = false;
    let mut has_video = false;
    let mut tracks = 0usize;
    for trak in moov.iter().filter(|item| &item.kind == b"trak") {
        match bmff_track_kind(trak.payload, fragmented)? {
            kind if &kind == b"soun" => has_audio = true,
            kind if &kind == b"vide" => has_video = true,
            _ => return None,
        }
        tracks += 1;
    }
    let has_media = top.iter().filter(|item| &item.kind == b"mdat").any(|item| {
        item.payload.len() >= 4 && item.payload.iter().any(|byte| *byte != 0)
    });
    (tracks > 0 && (has_audio || has_video) && has_media).then_some(BmffInfo {
        brand,
        has_audio,
        has_video,
    })
}

fn ebml_size(bytes: &[u8]) -> Option<(usize, usize, bool)> {
    let first = *bytes.first()?;
    let length = (first.leading_zeros() as usize).checked_add(1)?;
    if length > 8 || bytes.len() < length {
        return None;
    }
    let marker = 1_u8 << (8 - length);
    let mut value = usize::from(first & (marker - 1));
    for byte in &bytes[1..length] {
        value = value.checked_shl(8)?.checked_add(usize::from(*byte))?;
    }
    let unknown = value == ((1_u64 << (length * 7)) - 1) as usize;
    Some((value, length, unknown))
}

#[derive(Clone, Copy)]
struct EbmlElement<'a> {
    id: u32,
    payload: &'a [u8],
}

#[derive(Clone, Copy)]
struct WebmInfo {
    has_audio: bool,
    has_video: bool,
}

fn ebml_id(bytes: &[u8]) -> Option<(u32, usize)> {
    let first = *bytes.first()?;
    let length = (first.leading_zeros() as usize).checked_add(1)?;
    if length > 4 || bytes.len() < length {
        return None;
    }
    let mut id = 0u32;
    for byte in &bytes[..length] {
        id = id.checked_shl(8)?.checked_add(u32::from(*byte))?;
    }
    Some((id, length))
}

fn ebml_elements(bytes: &[u8]) -> Option<Vec<EbmlElement<'_>>> {
    let mut offset = 0usize;
    let mut elements = Vec::new();
    while offset < bytes.len() {
        if elements.len() >= MAX_CONTAINER_ELEMENTS {
            return None;
        }
        let (id, id_len) = ebml_id(&bytes[offset..])?;
        let size_offset = offset.checked_add(id_len)?;
        let (size, size_len, unknown) = ebml_size(&bytes[size_offset..])?;
        if unknown {
            return None;
        }
        let payload_start = size_offset.checked_add(size_len)?;
        let end = payload_start.checked_add(size)?;
        elements.push(EbmlElement {
            id,
            payload: bytes.get(payload_start..end)?,
        });
        offset = end;
    }
    (offset == bytes.len()).then_some(elements)
}

fn webm_track_type(payload: &[u8]) -> Option<u8> {
    let Some(elements) = ebml_elements(payload) else {
        return None;
    };
    let positive_uint = |id| {
        elements.iter().find(|item| item.id == id).is_some_and(|item| {
            !item.payload.is_empty()
                && item.payload.len() <= 8
                && item.payload.iter().fold(0u64, |value, byte| (value << 8) | u64::from(*byte)) > 0
        })
    };
    let track_type = elements
        .iter()
        .find(|item| item.id == 0x83)
        .and_then(|item| (item.payload.len() == 1 && matches!(item.payload[0], 1 | 2)).then_some(item.payload[0]));
    let valid_codec = elements.iter().find(|item| item.id == 0x86).is_some_and(|item| {
        !item.payload.is_empty() && item.payload.len() <= 128 && item.payload.iter().all(u8::is_ascii_graphic)
    });
    (positive_uint(0xd7) && valid_codec).then_some(track_type?)
}

fn valid_webm_block(payload: &[u8]) -> bool {
    let Some((track, track_len, unknown)) = ebml_size(payload) else {
        return false;
    };
    if unknown || track == 0 {
        return false;
    }
    let frame_start = track_len.saturating_add(3);
    payload.len() > frame_start && payload[frame_start..].iter().any(|byte| *byte != 0)
}

fn webm_info(bytes: &[u8]) -> Option<WebmInfo> {
    let Some(top) = ebml_elements(bytes) else {
        return None;
    };
    if top.len() != 2 || top[0].id != 0x1a45dfa3 || top[1].id != 0x18538067 {
        return None;
    }
    let Some(header) = ebml_elements(top[0].payload) else {
        return None;
    };
    if !header.iter().any(|item| item.id == 0x4282 && item.payload == b"webm") {
        return None;
    }
    let Some(segment) = ebml_elements(top[1].payload) else {
        return None;
    };
    let valid_info = segment.iter().find(|item| item.id == 0x1549a966).is_some_and(|item| {
        !item.payload.is_empty() && ebml_elements(item.payload).is_some_and(|children| !children.is_empty())
    });
    let track_types = segment
        .iter()
        .find(|item| item.id == 0x1654ae6b)
        .and_then(|item| ebml_elements(item.payload))?
        .into_iter()
        .filter(|entry| entry.id == 0xae)
        .map(|entry| webm_track_type(entry.payload))
        .collect::<Option<Vec<_>>>()?;
    let has_video = track_types.contains(&1);
    let has_audio = track_types.contains(&2);
    let valid_cluster = segment.iter().filter(|item| item.id == 0x1f43b675).any(|item| {
        ebml_elements(item.payload).is_some_and(|cluster| {
            cluster.iter().any(|child| {
                (child.id == 0xa3 && valid_webm_block(child.payload))
                    || (child.id == 0xa0
                        && ebml_elements(child.payload).is_some_and(|group| {
                            group.iter().any(|block| block.id == 0xa1 && valid_webm_block(block.payload))
                        }))
            })
        })
    });
    (valid_info && !track_types.is_empty() && valid_cluster).then_some(WebmInfo {
        has_audio,
        has_video,
    })
}

fn valid_wav(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return false;
    }
    let Some(riff_len) = read_le_u32(&bytes[4..]).and_then(|size| usize::try_from(size).ok()) else {
        return false;
    };
    let Some(end) = riff_len.checked_add(8) else {
        return false;
    };
    if end != bytes.len() || end < 12 {
        return false;
    }
    let (mut offset, mut block_align, mut has_data) = (12usize, None, false);
    while offset + 8 <= end {
        let id = &bytes[offset..offset + 4];
        let Some(size) = read_le_u32(&bytes[offset + 4..]).and_then(|size| usize::try_from(size).ok()) else {
            return false;
        };
        let Some(chunk_end) = offset.checked_add(8).and_then(|value| value.checked_add(size)) else {
            return false;
        };
        if chunk_end > end {
            return false;
        }
        if id == b"fmt " && size >= 16 && block_align.is_none() {
            let Some(format) = bytes.get(offset + 8..offset + 8 + 16) else {
                return false;
            };
            let codec = u16::from_le_bytes(format[..2].try_into().expect("WAV codec exists"));
            let channels = u16::from_le_bytes(format[2..4].try_into().expect("WAV channels exist"));
            let sample_rate = u32::from_le_bytes(format[4..8].try_into().expect("WAV rate exists"));
            let byte_rate = u32::from_le_bytes(format[8..12].try_into().expect("WAV byte rate exists"));
            let align = u16::from_le_bytes(format[12..14].try_into().expect("WAV alignment exists"));
            let bits = u16::from_le_bytes(format[14..16].try_into().expect("WAV sample size exists"));
            let expected_align = u32::from(channels).checked_mul(u32::from(bits).div_ceil(8));
            if !matches!(codec, 1 | 3 | 6 | 7 | 0xfffe)
                || channels == 0
                || channels > 32
                || !(1..=768_000).contains(&sample_rate)
                || bits == 0
                || bits > 64
                || align == 0
                || expected_align != Some(u32::from(align))
                || sample_rate.checked_mul(u32::from(align)) != Some(byte_rate)
            {
                return false;
            }
            block_align = Some(usize::from(align));
        } else if id == b"fmt " {
            return false;
        }
        if id == b"data" {
            let Some(align) = block_align else {
                return false;
            };
            if has_data || size == 0 || size % align != 0 {
                return false;
            }
            has_data = true;
        }
        let Some(next) = chunk_end.checked_add(size & 1) else {
            return false;
        };
        if next > end || (size & 1 == 1 && bytes.get(chunk_end) != Some(&0)) {
            return false;
        }
        offset = next;
    }
    block_align.is_some() && has_data && offset == end
}

#[derive(Clone, Copy)]
enum OggCodec {
    Opus,
    Vorbis,
    Speex,
    Flac,
}

fn ogg_codec_header(packet: &[u8]) -> Option<OggCodec> {
    if packet.starts_with(b"OpusHead") && packet.len() >= 19 && (1..=15).contains(&packet[8]) && packet[9] > 0 {
        Some(OggCodec::Opus)
    } else if packet.len() >= 30
        && packet[0] == 1
        && packet.get(1..7) == Some(b"vorbis")
        && packet[7..11] == [0, 0, 0, 0]
        && packet[11] > 0
        && u32::from_le_bytes(packet[12..16].try_into().ok()?) > 0
        && packet[28] & 0x0f >= 6
        && packet[28] >> 4 >= packet[28] & 0x0f
        && packet[28] >> 4 <= 13
        && packet[29] & 1 == 1
    {
        Some(OggCodec::Vorbis)
    } else if packet.starts_with(b"Speex   ") && packet.len() >= 80 {
        Some(OggCodec::Speex)
    } else if packet.starts_with(b"\x7fFLAC") && packet.len() >= 9 {
        Some(OggCodec::Flac)
    } else {
        None
    }
}

fn ogg_packet_is_audio(codec: OggCodec, packet_index: usize, packet: &[u8]) -> bool {
    if packet.is_empty() || packet.iter().all(|byte| *byte == 0) {
        return false;
    }
    match codec {
        OggCodec::Opus => !packet.starts_with(b"OpusHead") && !packet.starts_with(b"OpusTags"),
        OggCodec::Vorbis => {
            !(packet.len() >= 7 && matches!(packet[0], 1 | 3 | 5) && packet.get(1..7) == Some(b"vorbis"))
                && packet[0] & 1 == 0
        }
        OggCodec::Speex => packet_index >= 2,
        OggCodec::Flac => packet.len() >= 2 && packet[0] == 0xff && packet[1] & 0xfe == 0xf8,
    }
}

fn valid_ogg(bytes: &[u8]) -> bool {
    let (mut offset, mut pages) = (0usize, 0usize);
    let mut serial = None;
    let mut expected_sequence = 0u32;
    let mut packet = Vec::new();
    let mut codec = None;
    let mut packet_index = 0usize;
    let mut saw_audio = false;
    let mut saw_end = false;
    while offset < bytes.len() {
        let Some(header) = bytes.get(offset..offset + 27) else {
            return false;
        };
        if &header[..4] != b"OggS" || header[4] != 0 {
            return false;
        }
        let flags = header[5];
        let segment_count = header[26] as usize;
        if segment_count == 0
            || (pages == 0 && (flags & 0x02 == 0 || flags & 0x01 != 0))
            || (pages > 0 && flags & 0x02 != 0)
            || (flags & 0x01 != 0) != !packet.is_empty()
            || saw_end
        {
            return false;
        }
        let page_serial = u32::from_le_bytes(header[14..18].try_into().expect("Ogg serial exists"));
        let sequence = u32::from_le_bytes(header[18..22].try_into().expect("Ogg sequence exists"));
        if serial.is_some_and(|known| known != page_serial) || sequence != expected_sequence {
            return false;
        }
        serial.get_or_insert(page_serial);
        expected_sequence = expected_sequence.wrapping_add(1);
        let Some(lacing) = bytes.get(offset + 27..offset + 27 + segment_count) else {
            return false;
        };
        let payload_len: usize = lacing.iter().map(|value| *value as usize).sum();
        let Some(next) = offset
            .checked_add(27 + segment_count)
            .and_then(|value| value.checked_add(payload_len))
        else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        let page = &bytes[offset..next];
        let stored_crc = u32::from_le_bytes(page[22..26].try_into().expect("Ogg CRC field exists"));
        if ogg_crc(page) != stored_crc {
            return false;
        }
        let mut payload_offset = offset + 27 + segment_count;
        for lace in lacing {
            let end = payload_offset + usize::from(*lace);
            if packet.len().saturating_add(usize::from(*lace)) > MAX_OGG_PACKET_BYTES {
                return false;
            }
            packet.extend_from_slice(&bytes[payload_offset..end]);
            payload_offset = end;
            if *lace < 255 {
                if let Some(known) = codec {
                    saw_audio |= ogg_packet_is_audio(known, packet_index, &packet);
                } else {
                    codec = ogg_codec_header(&packet);
                    if codec.is_none() {
                        return false;
                    }
                }
                packet_index += 1;
                packet.clear();
            }
        }
        saw_end = flags & 0x04 != 0;
        if saw_end {
            let granule = u64::from_le_bytes(header[6..14].try_into().expect("Ogg granule exists"));
            if granule == 0 || granule == u64::MAX || next != bytes.len() {
                return false;
            }
        }
        offset = next;
        pages += 1;
    }
    pages > 0 && codec.is_some() && saw_audio && saw_end && packet.is_empty()
}

fn ogg_crc(bytes: &[u8]) -> u32 {
    let mut crc = 0_u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let byte = if (22..26).contains(&index) { 0 } else { byte };
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read(&mut self, bits: usize) -> Option<u64> {
        if bits > 64 || self.bit.checked_add(bits)? > self.bytes.len().checked_mul(8)? {
            return None;
        }
        let mut value = 0u64;
        for _ in 0..bits {
            value = (value << 1) | u64::from((self.bytes[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(value)
    }

    fn skip(&mut self, bits: usize) -> Option<()> {
        self.bit = self.bit.checked_add(bits)?;
        (self.bit <= self.bytes.len().checked_mul(8)?).then_some(())
    }

    fn unary(&mut self) -> Option<usize> {
        let mut zeros = 0usize;
        while self.read(1)? == 0 {
            zeros = zeros.checked_add(1)?;
            if zeros > 32 * 1024 * 1024 {
                return None;
            }
        }
        Some(zeros)
    }

    fn align_zero(&mut self) -> Option<()> {
        while self.bit % 8 != 0 {
            if self.read(1)? != 0 {
                return None;
            }
        }
        Some(())
    }

    fn byte_offset(&self) -> usize {
        self.bit / 8
    }
}

fn flac_crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn flac_crc16_update(mut crc: u16, byte: u8) -> u16 {
    crc ^= u16::from(byte) << 8;
    for _ in 0..8 {
        crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
    }
    crc
}

fn flac_utf8_number(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let first = *bytes.get(*offset)?;
    let (length, mut value, minimum) = if first & 0x80 == 0 {
        (1usize, u64::from(first), 0u64)
    } else {
        let leading = first.leading_ones() as usize;
        if !(2..=6).contains(&leading) {
            return None;
        }
        (leading, u64::from(first & (0x7f >> leading)), 1u64 << (5 * leading - 4))
    };
    for index in 1..length {
        let next = *bytes.get(offset.checked_add(index)?)?;
        if next & 0xc0 != 0x80 {
            return None;
        }
        value = value.checked_shl(6)?.checked_add(u64::from(next & 0x3f))?;
    }
    if length > 1 && value < minimum {
        return None;
    }
    *offset = offset.checked_add(length)?;
    Some(value)
}

fn flac_residual(reader: &mut BitReader<'_>, block_size: usize, predictor_order: usize) -> Option<()> {
    let method = reader.read(2)? as usize;
    if method > 1 {
        return None;
    }
    let parameter_bits = if method == 0 { 4 } else { 5 };
    let escape = (1usize << parameter_bits) - 1;
    let partition_order = reader.read(4)? as usize;
    let partitions = 1usize.checked_shl(u32::try_from(partition_order).ok()?)?;
    if partitions == 0 || block_size % partitions != 0 {
        return None;
    }
    let partition_samples = block_size / partitions;
    for partition in 0..partitions {
        let samples = if partition == 0 {
            partition_samples.checked_sub(predictor_order)?
        } else {
            partition_samples
        };
        let parameter = reader.read(parameter_bits)? as usize;
        if parameter == escape {
            let raw_bits = reader.read(5)? as usize;
            reader.skip(samples.checked_mul(raw_bits)?)?;
        } else {
            for _ in 0..samples {
                reader.unary()?;
                reader.skip(parameter)?;
            }
        }
    }
    Some(())
}

fn flac_subframe(reader: &mut BitReader<'_>, block_size: usize, bits_per_sample: usize) -> Option<()> {
    if reader.read(1)? != 0 {
        return None;
    }
    let kind = reader.read(6)? as usize;
    let wasted = if reader.read(1)? == 1 { reader.unary()?.checked_add(1)? } else { 0 };
    let sample_bits = bits_per_sample.checked_sub(wasted)?;
    if sample_bits == 0 || sample_bits > 64 {
        return None;
    }
    match kind {
        0 => reader.skip(sample_bits),
        1 => reader.skip(block_size.checked_mul(sample_bits)?),
        8..=12 => {
            let order = kind - 8;
            reader.skip(order.checked_mul(sample_bits)?)?;
            flac_residual(reader, block_size, order)
        }
        32..=63 => {
            let order = (kind & 31) + 1;
            reader.skip(order.checked_mul(sample_bits)?)?;
            let precision = reader.read(4)? as usize;
            if precision == 15 {
                return None;
            }
            reader.skip(5)?;
            reader.skip(order.checked_mul(precision + 1)?)?;
            flac_residual(reader, block_size, order)
        }
        _ => None,
    }
}

fn parse_flac_frame(
    bytes: &[u8],
    stream_bits_per_sample: usize,
    min_block_size: usize,
    max_block_size: usize,
) -> Option<(usize, usize)> {
    if bytes.get(0) != Some(&0xff)
        || bytes.get(1).is_none_or(|byte| byte & 0xfe != 0xf8)
        || bytes.get(1).is_some_and(|byte| byte & 0x02 != 0)
    {
        return None;
    }
    let block_code = usize::from(*bytes.get(2)? >> 4);
    let sample_rate_code = usize::from(*bytes.get(2)? & 0x0f);
    let channel_assignment = usize::from(*bytes.get(3)? >> 4);
    let sample_size_code = usize::from((*bytes.get(3)? >> 1) & 0x07);
    if block_code == 0 || sample_rate_code == 15 || channel_assignment > 10 || bytes[3] & 1 != 0 {
        return None;
    }
    let mut offset = 4usize;
    flac_utf8_number(bytes, &mut offset)?;
    let block_size = match block_code {
        1 => 192,
        2..=5 => 576usize.checked_shl(u32::try_from(block_code - 2).ok()?)?,
        6 => {
            let value = usize::from(*bytes.get(offset)?) + 1;
            offset += 1;
            value
        }
        7 => {
            let value = usize::from(u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?)) + 1;
            offset += 2;
            value
        }
        8..=15 => 256usize.checked_shl(u32::try_from(block_code - 8).ok()?)?,
        _ => return None,
    };
    match sample_rate_code {
        12 => {
            if *bytes.get(offset)? == 0 {
                return None;
            }
            offset += 1;
        }
        13 | 14 => {
            if u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) == 0 {
                return None;
            }
            offset += 2;
        }
        _ => {}
    }
    let bits_per_sample = match sample_size_code {
        0 => stream_bits_per_sample,
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        _ => return None,
    };
    if block_size < min_block_size || block_size > max_block_size || flac_crc8(bytes.get(..offset)?) != *bytes.get(offset)? {
        return None;
    }
    let frame_data = offset + 1;
    let channels = if channel_assignment <= 7 { channel_assignment + 1 } else { 2 };
    let mut reader = BitReader::new(bytes.get(frame_data..)?);
    for channel in 0..channels {
        let extra = usize::from(
            (channel_assignment == 8 && channel == 1)
                || (channel_assignment == 9 && channel == 0)
                || (channel_assignment == 10 && channel == 1),
        );
        flac_subframe(&mut reader, block_size, bits_per_sample.checked_add(extra)?)?;
    }
    reader.align_zero()?;
    let crc_offset = frame_data.checked_add(reader.byte_offset())?;
    let stored = u16::from_be_bytes(bytes.get(crc_offset..crc_offset + 2)?.try_into().ok()?);
    let calculated = bytes[..crc_offset].iter().fold(0u16, |crc, byte| flac_crc16_update(crc, *byte));
    (calculated == stored).then_some((crc_offset + 2, block_size))
}

fn valid_flac(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"fLaC") {
        return false;
    }
    let (mut offset, mut first, mut final_block) = (4usize, true, false);
    let mut stream = None;
    while !final_block {
        let Some(header) = bytes.get(offset..offset + 4) else {
            return false;
        };
        final_block = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7f;
        let len = ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | header[3] as usize;
        if block_type == 127 || (first && (block_type != 0 || len != 34)) {
            return false;
        }
        let Some(next) = offset.checked_add(4).and_then(|value| value.checked_add(len)) else {
            return false;
        };
        let Some(payload) = bytes.get(offset + 4..next) else {
            return false;
        };
        if first {
            let min_block = usize::from(u16::from_be_bytes(payload[0..2].try_into().expect("STREAMINFO block size")));
            let max_block = usize::from(u16::from_be_bytes(payload[2..4].try_into().expect("STREAMINFO block size")));
            let min_frame = ((payload[4] as usize) << 16) | ((payload[5] as usize) << 8) | payload[6] as usize;
            let max_frame = ((payload[7] as usize) << 16) | ((payload[8] as usize) << 8) | payload[9] as usize;
            let word = u64::from_be_bytes(payload[10..18].try_into().expect("STREAMINFO stream fields"));
            let sample_rate = (word >> 44) & 0x0f_ffff;
            let channels = ((word >> 41) & 0x07) + 1;
            let bits_per_sample = ((word >> 36) & 0x1f) + 1;
            let total_samples = word & 0x0f_ffff_ffff;
            if min_block < 16
                || max_block < min_block
                || (min_frame > 0 && max_frame > 0 && max_frame < min_frame)
                || sample_rate == 0
                || channels > 8
                || !(4..=32).contains(&bits_per_sample)
                || total_samples == 0
            {
                return false;
            }
            stream = Some((min_block, max_block, bits_per_sample as usize, total_samples));
        }
        first = false;
        offset = next;
    }
    let Some((min_block, max_block, bits_per_sample, total_samples)) = stream else {
        return false;
    };
    let mut frames = 0usize;
    let mut decoded_samples = 0u64;
    while offset < bytes.len() {
        let Some((consumed, block_size)) = parse_flac_frame(&bytes[offset..], bits_per_sample, min_block, max_block) else {
            return false;
        };
        if consumed == 0 {
            return false;
        }
        offset += consumed;
        decoded_samples = match decoded_samples.checked_add(block_size as u64) {
            Some(value) => value,
            None => return false,
        };
        frames += 1;
    }
    frames > 0 && decoded_samples == total_samples
}

fn mp3_frame_len(header: &[u8]) -> Option<usize> {
    if header.len() < 4 || header[0] != 0xff || (header[1] & 0xe0) != 0xe0 {
        return None;
    }
    let version = (header[1] >> 3) & 0x03;
    let layer = (header[1] >> 1) & 0x03;
    let bitrate_index = (header[2] >> 4) as usize;
    let sample_index = ((header[2] >> 2) & 0x03) as usize;
    if version == 1 || layer == 0 || !(1..15).contains(&bitrate_index) || sample_index == 3 {
        return None;
    }
    const MPEG1_LAYER1: [usize; 16] = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0];
    const MPEG1_LAYER2: [usize; 16] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0];
    const MPEG1_LAYER3: [usize; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const MPEG2_LAYER1: [usize; 16] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0];
    const MPEG2_OTHER: [usize; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    let bitrate_kbps = match (version == 3, layer) {
        (true, 3) => MPEG1_LAYER1[bitrate_index],
        (true, 2) => MPEG1_LAYER2[bitrate_index],
        (true, 1) => MPEG1_LAYER3[bitrate_index],
        (false, 3) => MPEG2_LAYER1[bitrate_index],
        (false, _) => MPEG2_OTHER[bitrate_index],
        _ => return None,
    };
    let base_sample_rate = [44_100usize, 48_000, 32_000][sample_index];
    let sample_rate = match version {
        3 => base_sample_rate,
        2 => base_sample_rate / 2,
        0 => base_sample_rate / 4,
        _ => return None,
    };
    let padding = ((header[2] >> 1) & 1) as usize;
    let bitrate = bitrate_kbps.checked_mul(1_000)?;
    let frame_len = if layer == 3 {
        (12 * bitrate / sample_rate + padding) * 4
    } else if layer == 1 && version != 3 {
        72 * bitrate / sample_rate + padding
    } else {
        144 * bitrate / sample_rate + padding
    };
    (frame_len >= 4).then_some(frame_len)
}

fn valid_mp3(bytes: &[u8]) -> bool {
    let mut offset = 0usize;
    if bytes.starts_with(b"ID3") {
        let Some(header) = bytes.get(..10) else {
            return false;
        };
        let size = &header[6..10];
        if size.iter().any(|byte| byte & 0x80 != 0) {
            return false;
        }
        let tag_len = size.iter().fold(0usize, |value, byte| (value << 7) | *byte as usize);
        let footer_len = if header[3] == 4 && header[5] & 0x10 != 0 { 10 } else { 0 };
        let Some(next) = 10usize.checked_add(tag_len).and_then(|value| value.checked_add(footer_len)) else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        offset = next;
    }
    let mut frames = 0usize;
    let mut stream_signature = None;
    while offset < bytes.len() {
        if bytes.len() - offset == 128 && bytes.get(offset..offset + 3) == Some(b"TAG") {
            offset = bytes.len();
            break;
        }
        let Some(frame_len) = mp3_frame_len(&bytes[offset..]) else {
            return false;
        };
        let header = &bytes[offset..offset + 4];
        let signature = (header[1] & 0x1e, header[2] & 0x0c, header[3] & 0xc0);
        if stream_signature.is_some_and(|known| known != signature) {
            return false;
        }
        stream_signature.get_or_insert(signature);
        let Some(next) = offset.checked_add(frame_len) else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        let payload_start = offset + if header[1] & 1 == 0 { 6 } else { 4 };
        if payload_start >= next || bytes[payload_start..next].iter().all(|byte| *byte == 0) {
            return false;
        }
        offset = next;
        frames += 1;
    }
    frames >= 2 && offset == bytes.len()
}

fn detect_video_mime(bytes: &[u8]) -> Option<&'static str> {
    if let Some(info) = iso_bmff_info(bytes).filter(|info| info.has_video) {
        Some(if &info.brand == b"qt  " { "video/quicktime" } else { "video/mp4" })
    } else if webm_info(bytes).is_some_and(|info| info.has_video) {
        Some("video/webm")
    } else {
        None
    }
}

fn detect_audio_mime(bytes: &[u8]) -> Option<&'static str> {
    if valid_wav(bytes) {
        Some("audio/wav")
    } else if valid_ogg(bytes) {
        Some("audio/ogg")
    } else if valid_flac(bytes) {
        Some("audio/flac")
    } else if valid_mp3(bytes) {
        Some("audio/mpeg")
    } else if webm_info(bytes).is_some_and(|info| info.has_audio) {
        Some("audio/webm")
    } else if iso_bmff_info(bytes).is_some_and(|info| info.has_audio) {
        Some("audio/mp4")
    } else {
        None
    }
}

fn validate_detected_binary(
    bytes: &[u8],
    declared_mime: Option<&str>,
    expected_kind: &str,
    detected_mime: Option<&'static str>,
) -> Result<String, CreationError> {
    reject_binary_error_document(bytes, declared_mime)?;
    let detected = detected_mime.ok_or_else(|| {
        invalid(format!(
            "provider returned unrecognized or corrupt {expected_kind} bytes"
        ))
    })?;
    if let Some(declared) = declared_mime.filter(|mime| !is_generic_binary_mime(mime))
        && declared != detected
    {
        return Err(invalid(format!(
            "artifact MIME mismatch: declared '{declared}', detected '{detected}'"
        )));
    }
    Ok(detected.to_string())
}

fn validate_video(bytes: &[u8], declared_mime: Option<&str>) -> Result<String, CreationError> {
    validate_detected_binary(bytes, declared_mime, "video", detect_video_mime(bytes))
}

fn validate_audio(bytes: &[u8], declared_mime: Option<&str>) -> Result<String, CreationError> {
    validate_detected_binary(bytes, declared_mime, "audio", detect_audio_mime(bytes))
}

/// Reconcile an adapter MIME hint with the HTTP response Content-Type. Generic
/// binary Content-Types do not override a useful hint; two conflicting
/// concrete declarations are rejected instead of silently trusting one.
pub(crate) fn reconcile_mime(
    hint: Option<&str>,
    response_content_type: Option<&str>,
) -> Result<Option<String>, CreationError> {
    let hint = hint.and_then(normalize_mime);
    let response = response_content_type.and_then(normalize_mime);
    match (hint, response) {
        (Some(hint), Some(response)) if is_generic_binary_mime(&response) => Ok(Some(hint)),
        (Some(hint), Some(response)) if is_generic_binary_mime(&hint) => Ok(Some(response)),
        (Some(hint), Some(response)) if hint != response => Err(invalid(format!(
            "artifact MIME mismatch: adapter declared '{hint}', HTTP response declared '{response}'"
        ))),
        (Some(mime), Some(_)) | (Some(mime), None) | (None, Some(mime)) => Ok(Some(mime)),
        (None, None) => Ok(None),
    }
}

/// Validate and normalize an artifact against the capability that produced it.
/// This is the service-level gate used before an asset sink is invoked.
pub(crate) fn validate_for_capability(
    bytes: &[u8],
    declared_mime: Option<&str>,
    capability: MediaCapability,
) -> Result<String, CreationError> {
    let declared = declared_mime.and_then(normalize_mime);
    match capability {
        MediaCapability::T2i | MediaCapability::I2i | MediaCapability::Inpaint => {
            if let Some(mime) = declared.as_deref()
                && !is_generic_binary_mime(mime)
                && !mime.starts_with("image/")
            {
                return Err(invalid(format!(
                    "artifact MIME mismatch: image task received '{mime}'"
                )));
            }
            validate_image(bytes, declared.as_deref())
        }
        MediaCapability::T2v | MediaCapability::I2v | MediaCapability::V2v => {
            validate_video(bytes, declared.as_deref())
        }
        MediaCapability::Tts => validate_audio(bytes, declared.as_deref()),
        MediaCapability::Text => validate_text(bytes, declared.as_deref()),
    }
}

/// Defense-in-depth validation for asset sinks that receive already-resolved
/// bytes and MIME. Returns the canonical MIME to index and use for extensions.
pub fn validate_artifact_payload(bytes: &[u8], mime: &str) -> Result<String, CreationError> {
    let declared = normalize_mime(mime).ok_or_else(|| invalid("artifact MIME is empty"))?;
    if declared.starts_with("image/") {
        validate_image(bytes, Some(&declared))
    } else if declared.starts_with("video/") {
        validate_video(bytes, Some(&declared))
    } else if declared.starts_with("audio/") {
        validate_audio(bytes, Some(&declared))
    } else if declared == "text/plain" {
        validate_text(bytes, Some(&declared))
    } else {
        Err(invalid(format!("unsupported artifact MIME '{declared}'")))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    fn png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn bmff_box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(payload.len() + 8);
        output.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
        output.extend_from_slice(kind);
        output.extend_from_slice(payload);
        output
    }

    pub(crate) fn bmff(brand: &[u8; 4]) -> Vec<u8> {
        let handler = if brand == b"M4A " { b"soun" } else { b"vide" };
        let sample_entry = bmff_box_bytes(if handler == b"soun" { b"mp4a" } else { b"avc1" }, &[1; 8]);
        let mut stsd = vec![0; 4];
        stsd.extend_from_slice(&1u32.to_be_bytes());
        stsd.extend_from_slice(&sample_entry);
        let mut stts = vec![0; 4];
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&1u32.to_be_bytes());
        let mut stsc = vec![0; 4];
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        let mut stsz = vec![0; 4];
        stsz.extend_from_slice(&4u32.to_be_bytes());
        stsz.extend_from_slice(&1u32.to_be_bytes());
        let mut stco = vec![0; 4];
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&1u32.to_be_bytes());
        let stbl = [
            bmff_box_bytes(b"stsd", &stsd),
            bmff_box_bytes(b"stts", &stts),
            bmff_box_bytes(b"stsc", &stsc),
            bmff_box_bytes(b"stsz", &stsz),
            bmff_box_bytes(b"stco", &stco),
        ]
        .concat();
        let minf = bmff_box_bytes(b"stbl", &stbl);
        let mut hdlr = vec![0; 12];
        hdlr[8..12].copy_from_slice(handler);
        let mdia = [
            bmff_box_bytes(b"mdhd", &[1; 8]),
            bmff_box_bytes(b"hdlr", &hdlr),
            bmff_box_bytes(b"minf", &minf),
        ]
        .concat();
        let trak = [
            bmff_box_bytes(b"tkhd", &[1; 8]),
            bmff_box_bytes(b"mdia", &mdia),
        ]
        .concat();
        let moov = [
            bmff_box_bytes(b"mvhd", &[1; 8]),
            bmff_box_bytes(b"trak", &trak),
        ]
        .concat();
        let mut ftyp = brand.to_vec();
        ftyp.extend_from_slice(&[0; 4]);
        [
            bmff_box_bytes(b"ftyp", &ftyp),
            bmff_box_bytes(b"moov", &moov),
            bmff_box_bytes(b"mdat", &[1, 2, 3, 4]),
        ]
        .concat()
    }

    fn wav() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&38u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 1, 0]);
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 8, 0]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[128, 0]);
        bytes
    }

    fn ogg() -> Vec<u8> {
        let mut opus = b"OpusHead".to_vec();
        opus.extend_from_slice(&[1, 1, 0, 0]);
        opus.extend_from_slice(&48_000u32.to_le_bytes());
        opus.extend_from_slice(&[0, 0, 0]);
        let audio = [0xf8, 1, 2, 3, 4];
        let mut bytes = vec![0; 27];
        bytes[..4].copy_from_slice(b"OggS");
        bytes[5] = 0x06;
        bytes[6..14].copy_from_slice(&960u64.to_le_bytes());
        bytes[14..18].copy_from_slice(&1u32.to_le_bytes());
        bytes[26] = 2;
        bytes.extend_from_slice(&[opus.len() as u8, audio.len() as u8]);
        bytes.extend_from_slice(&opus);
        bytes.extend_from_slice(&audio);
        let crc = ogg_crc(&bytes);
        bytes[22..26].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    fn flac() -> Vec<u8> {
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0x80, 0, 0, 34]);
        let mut stream_info = [0u8; 34];
        stream_info[..2].copy_from_slice(&16u16.to_be_bytes());
        stream_info[2..4].copy_from_slice(&16u16.to_be_bytes());
        let stream_word = (8_000u64 << 44) | (7u64 << 36) | 16;
        stream_info[10..18].copy_from_slice(&stream_word.to_be_bytes());
        bytes.extend_from_slice(&stream_info);
        let mut frame = vec![0xff, 0xf8, 0x60, 0x02, 0, 15];
        let header_crc = flac_crc8(&frame);
        frame.push(header_crc);
        frame.extend_from_slice(&[0, 1]);
        let crc = frame.iter().fold(0u16, |crc, byte| flac_crc16_update(crc, *byte));
        frame.extend_from_slice(&crc.to_be_bytes());
        bytes.extend_from_slice(&frame);
        bytes
    }

    fn mp3() -> Vec<u8> {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        frame[10] = 1;
        [frame.clone(), frame].concat()
    }

    fn ebml_element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 127);
        let mut output = id.to_vec();
        output.push(0x80 | payload.len() as u8);
        output.extend_from_slice(payload);
        output
    }

    fn webm() -> Vec<u8> {
        let header = ebml_element(&[0x42, 0x82], b"webm");
        let info = ebml_element(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]);
        let track = [
            ebml_element(&[0xd7], &[1]),
            ebml_element(&[0x83], &[1]),
            ebml_element(&[0x86], b"V_VP8"),
        ]
        .concat();
        let tracks = ebml_element(&[0xae], &track);
        let cluster = [
            ebml_element(&[0xe7], &[0]),
            ebml_element(&[0xa3], &[0x81, 0, 0, 0, 1]),
        ]
        .concat();
        let segment = [
            ebml_element(&[0x15, 0x49, 0xa9, 0x66], &info),
            ebml_element(&[0x16, 0x54, 0xae, 0x6b], &tracks),
            ebml_element(&[0x1f, 0x43, 0xb6, 0x75], &cluster),
        ]
        .concat();
        [
            ebml_element(&[0x1a, 0x45, 0xdf, 0xa3], &header),
            ebml_element(&[0x18, 0x53, 0x80, 0x67], &segment),
        ]
        .concat()
    }

    #[test]
    fn validates_and_normalizes_real_image() {
        assert_eq!(validate_artifact_payload(&png(), "IMAGE/PNG; charset=binary").unwrap(), "image/png");
    }

    #[test]
    fn rejects_empty_corrupt_html_and_mismatched_images() {
        assert_eq!(validate_artifact_payload(&[], "image/png").unwrap_err().kind, "invalid_artifact");
        assert!(validate_artifact_payload(b"not a png", "image/png").is_err());
        assert!(validate_artifact_payload(b"<!doctype html><title>502</title>", "image/png").is_err());
        assert!(validate_artifact_payload(&png(), "image/jpeg").is_err());
        assert!(validate_artifact_payload(br#"{"error":"upstream failed"}"#, "video/mp4").is_err());
        assert!(validate_artifact_payload(b"not really a video", "video/mp4").is_err());
        assert!(validate_artifact_payload(b"not really audio", "audio/mpeg").is_err());
    }

    #[test]
    fn validates_video_and_audio_signatures_instead_of_trusting_mime() {
        let mp4 = bmff(b"isom");
        let quicktime = bmff(b"qt  ");
        let m4a = bmff(b"M4A ");
        let wav = wav();
        assert_eq!(validate_artifact_payload(&mp4, "video/mp4").unwrap(), "video/mp4");
        assert_eq!(
            validate_artifact_payload(&quicktime, "video/quicktime").unwrap(),
            "video/quicktime"
        );
        assert_eq!(validate_artifact_payload(&m4a, "audio/mp4").unwrap(), "audio/mp4");
        assert_eq!(validate_artifact_payload(&wav, "audio/x-wav").unwrap(), "audio/wav");
        assert_eq!(validate_artifact_payload(&ogg(), "audio/ogg").unwrap(), "audio/ogg");
        assert_eq!(validate_artifact_payload(&flac(), "audio/flac").unwrap(), "audio/flac");
        assert_eq!(validate_artifact_payload(&mp3(), "audio/mpeg").unwrap(), "audio/mpeg");
        assert_eq!(validate_artifact_payload(&webm(), "video/webm").unwrap(), "video/webm");
        assert!(validate_artifact_payload(&mp4, "audio/mp4").is_err());
        assert!(validate_artifact_payload(&m4a, "video/mp4").is_err());
        assert!(validate_artifact_payload(&mp4, "video/webm").is_err());
        assert!(validate_artifact_payload(&wav, "audio/mpeg").is_err());
        assert!(validate_artifact_payload(&mp4[..12], "video/mp4").is_err());
        assert!(validate_artifact_payload(&wav[..12], "audio/wav").is_err());
    }

    #[test]
    fn rejects_container_shells_crc_failures_zero_payloads_and_truncation() {
        let mut bmff_shell = bmff_box_bytes(b"ftyp", b"isom\0\0\0\0");
        bmff_shell.extend_from_slice(&bmff_box_bytes(b"moov", &[0]));
        bmff_shell.extend_from_slice(&bmff_box_bytes(b"mdat", &[1]));
        assert!(validate_artifact_payload(&bmff_shell, "video/mp4").is_err());

        let webm_shell = [
            ebml_element(&[0x1a, 0x45, 0xdf, 0xa3], &ebml_element(&[0x42, 0x82], b"webm")),
            ebml_element(&[0x18, 0x53, 0x80, 0x67], &[0]),
        ]
        .concat();
        assert!(validate_artifact_payload(&webm_shell, "video/webm").is_err());

        let mut bad_ogg = ogg();
        bad_ogg[22] ^= 1;
        assert!(validate_artifact_payload(&bad_ogg, "audio/ogg").is_err());
        let mut bad_flac = flac();
        *bad_flac.last_mut().unwrap() ^= 1;
        assert!(validate_artifact_payload(&bad_flac, "audio/flac").is_err());
        let mut zero_mp3 = mp3();
        for frame in zero_mp3.chunks_mut(417) {
            frame[4..].fill(0);
        }
        assert!(validate_artifact_payload(&zero_mp3, "audio/mpeg").is_err());

        for (bytes, mime) in [
            (bmff(b"isom"), "video/mp4"),
            (webm(), "video/webm"),
            (wav(), "audio/wav"),
            (ogg(), "audio/ogg"),
            (flac(), "audio/flac"),
            (mp3(), "audio/mpeg"),
        ] {
            assert!(validate_artifact_payload(&bytes[..bytes.len() - 1], mime).is_err());
        }
    }

    #[test]
    fn malformed_media_corpus_never_panics() {
        let fixtures = [bmff(b"isom"), webm(), wav(), ogg(), flac(), mp3()];
        for fixture in &fixtures {
            for cut in 0..fixture.len() {
                let prefix = &fixture[..cut];
                assert!(std::panic::catch_unwind(|| {
                    let _ = iso_bmff_info(prefix);
                    let _ = webm_info(prefix);
                    let _ = valid_wav(prefix);
                    let _ = valid_ogg(prefix);
                    let _ = valid_flac(prefix);
                    let _ = valid_mp3(prefix);
                })
                .is_ok());
            }
        }
        let mut state = 0x5a17_4c3d_91e1_0da5u64;
        for length in 0..256usize {
            let mut bytes = vec![0u8; length];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            assert!(std::panic::catch_unwind(|| {
                let _ = iso_bmff_info(&bytes);
                let _ = webm_info(&bytes);
                let _ = valid_wav(&bytes);
                let _ = valid_ogg(&bytes);
                let _ = valid_flac(&bytes);
                let _ = valid_mp3(&bytes);
            })
            .is_ok());
        }
    }

    #[test]
    fn text_artifacts_may_legitimately_contain_html_or_json_source() {
        assert_eq!(
            validate_artifact_payload(b"<!doctype html><title>generated page</title>", "text/plain").unwrap(),
            "text/plain"
        );
        assert_eq!(validate_artifact_payload(br#"{"generated":true}"#, "text/plain").unwrap(), "text/plain");
    }

    #[test]
    fn conflicting_transport_mime_is_rejected() {
        assert!(reconcile_mime(Some("image/png"), Some("text/html")).is_err());
        assert_eq!(
            reconcile_mime(Some("image/png"), Some("application/octet-stream")).unwrap().as_deref(),
            Some("image/png")
        );
    }
}
