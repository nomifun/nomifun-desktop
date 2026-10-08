//! Agnes Video v2.0's established frame-based request contract.
//! Kept separate from Video 2.5's resolution-tier and media-mode vocabulary.
//! Restored from the working adapter before commit 8e3a0c0ea; model availability
//! is determined by the configured account's API response, not a docs label.

use nomifun_api_types::{AgnesModelContract, agnes_model_contract};
use serde_json::{Value, json};

use crate::adapters::json_request_body;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::types::VideoGenRequest;

use super::{VIDEO_ADAPTER_ID, image_data_uri};

const DEFAULT_WIDTH: u32 = 1152;
const DEFAULT_HEIGHT: u32 = 768;
const DEFAULT_FRAME_RATE: u32 = 24;
const DEFAULT_NUM_FRAMES: u32 = 121;
const MAX_NUM_FRAMES: u32 = 441;

fn invalid(message: impl Into<String>) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

fn request_u32(configured: &Value, extra: &Value, field: &str) -> Result<Option<u32>, InvokeError> {
    extra.get(field).or_else(|| configured.get(field)).map(|value| {
        value.as_u64().and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid(format!("Agnes Video v2.0 {field} must be a positive integer")))
    }).transpose()
}

fn dimensions(size: &str) -> Result<(u32, u32), InvokeError> {
    let size = size.trim().to_ascii_lowercase();
    let (width, height) = size.split_once('x')
        .ok_or_else(|| invalid("Agnes Video v2.0 size must be WIDTHxHEIGHT"))?;
    let parse = |value: &str| value.trim().parse::<u32>()
        .map_err(|_| invalid("Agnes Video v2.0 dimensions must be positive multiples of 8"));
    let (width, height) = (parse(width)?, parse(height)?);
    if width == 0 || height == 0 || width % 8 != 0 || height % 8 != 0 {
        return Err(invalid("Agnes Video v2.0 dimensions must be positive multiples of 8"));
    }
    Ok((width, height))
}

fn frames_for_seconds(seconds: u32, frame_rate: u32) -> Result<u32, InvokeError> {
    if seconds == 0 {
        return Err(invalid("Agnes Video v2.0 seconds must be positive"));
    }
    let raw = seconds.checked_mul(frame_rate)
        .ok_or_else(|| invalid("Agnes Video v2.0 duration is too large"))?;
    let groups = raw.saturating_sub(1).saturating_add(4) / 8;
    let frames = groups.checked_mul(8).and_then(|value| value.checked_add(1))
        .ok_or_else(|| invalid("Agnes Video v2.0 frame count is too large"))?;
    validate_num_frames(frames)
}

fn validate_num_frames(frames: u32) -> Result<u32, InvokeError> {
    if frames <= MAX_NUM_FRAMES && frames % 8 == 1 {
        Ok(frames)
    } else {
        Err(invalid(format!("Agnes Video v2.0 num_frames must be at most {MAX_NUM_FRAMES} and satisfy 8n+1")))
    }
}

pub(super) fn build_video_body(
    model: &str,
    configured: &Value,
    request: &VideoGenRequest,
) -> Result<Value, InvokeError> {
    if agnes_model_contract(model) != Some(AgnesModelContract::VideoV20) {
        return Err(invalid("Agnes Video v2.0 request builder requires agnes-video-v2.0"));
    }
    let (width, height) = match request.size.as_deref().map(str::trim).filter(|size| !size.is_empty()) {
        Some(size) => dimensions(size)?,
        None => {
            // Preserve saved/extra dimensions when the shared request is automatic.
            let width = request_u32(configured, &request.extra, "width")?.unwrap_or(DEFAULT_WIDTH);
            let height = request_u32(configured, &request.extra, "height")?.unwrap_or(DEFAULT_HEIGHT);
            dimensions(&format!("{width}x{height}"))?
        }
    };
    let frame_rate = request_u32(configured, &request.extra, "frame_rate")?.unwrap_or(DEFAULT_FRAME_RATE);
    if !(1..=60).contains(&frame_rate) {
        return Err(invalid("Agnes Video v2.0 frame_rate must be from 1 to 60"));
    }
    let num_frames = match request.seconds {
        Some(seconds) => frames_for_seconds(seconds, frame_rate)?,
        None => validate_num_frames(request_u32(configured, &request.extra, "num_frames")?
            .unwrap_or(DEFAULT_NUM_FRAMES))?,
    };
    for (index, input) in request.inputs.iter().enumerate() {
        let valid = match input.role.as_str() {
            "reference" | "image" => true,
            "first_frame" => index == 0,
            "last_frame" => request.inputs.len() > 1 && index + 1 == request.inputs.len(),
            _ => false,
        };
        if !valid {
            return Err(invalid("Agnes Video v2.0 keyframes require ordered images, with first_frame first and last_frame last"));
        }
    }
    let mut typed = json!({
        "model": model, "prompt": request.prompt,
        "width": width, "height": height, "num_frames": num_frames, "frame_rate": frame_rate,
    });
    let images = request.inputs.iter().enumerate()
        .map(|(index, input)| image_data_uri(input, index + 1, VIDEO_ADAPTER_ID))
        .collect::<Result<Vec<_>, _>>()?;
    if images.len() > 1 {
        typed["extra_body"] = json!({"image": images, "mode": "keyframes"});
    } else if let Some(image) = images.first() {
        typed["image"] = json!(image);
    }
    let mut body = json_request_body(configured, &request.extra, typed)?;
    if !request.inputs.is_empty() {
        // Typed assets must not be shadowed by saved single-image/keyframe defaults.
        body.as_object_mut().unwrap().remove("mode");
        if request.inputs.len() > 1 {
            body.as_object_mut().unwrap().remove("image");
        } else if let Some(extra) = body.get_mut("extra_body").and_then(Value::as_object_mut) {
            extra.remove("image");
            extra.remove("mode");
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InputAsset;

    fn request() -> VideoGenRequest {
        VideoGenRequest {
            prompt: "waves".into(), seconds: None, size: None, resolution: None,
            inputs: vec![], extra: json!({}),
        }
    }

    fn input(role: &str, bytes: &[u8]) -> InputAsset {
        InputAsset { id: None, role: role.into(), bytes: bytes.to_vec(), mime: "image/png".into() }
    }

    #[test]
    fn v20_defaults_and_frames_do_not_use_the_25_wire_contract() {
        assert_eq!(build_video_body("agnes-video-v2.0", &json!({}), &request()).unwrap(),
            json!({"model":"agnes-video-v2.0", "prompt":"waves", "width":1152, "height":768,
                "num_frames":121, "frame_rate":24}));
        for (seconds, frames) in [(5, 121), (10, 241), (15, 361)] {
            let mut request = request();
            request.seconds = Some(seconds);
            let body = build_video_body("agnes-video-v2.0", &json!({}), &request).unwrap();
            assert_eq!(body["num_frames"], frames);
            for field in ["seconds", "size", "aspect_ratio", "first_frame", "images"] {
                assert!(body.get(field).is_none());
            }
        }
        assert!(build_video_body("agnes-video-2.5", &json!({}), &request()).is_err());
    }

    #[test]
    fn v20_preserves_saved_extra_and_typed_dimensions_and_frame_settings() {
        let configured = json!({"width":1920, "height":1080, "frame_rate":16, "num_frames":161,
            "seed":1, "endpoint":"/not-a-body-field"});
        let mut request = request();
        let body = build_video_body("agnes-video-v2.0", &configured, &request).unwrap();
        assert_eq!(body["width"], 1920);
        assert_eq!(body["height"], 1080);
        assert_eq!(body["frame_rate"], 16);
        assert_eq!(body["num_frames"], 161);
        assert!(body.get("endpoint").is_none());
        request.extra = json!({"width":1280, "height":720, "num_frames":241, "seed":42});
        let body = build_video_body("agnes-video-v2.0", &configured, &request).unwrap();
        assert_eq!(body["width"], 1280);
        assert_eq!(body["height"], 720);
        assert_eq!(body["num_frames"], 241);
        assert_eq!(body["seed"], 42);
        request.size = Some("768x1024".into());
        request.seconds = Some(5);
        let body = build_video_body("agnes-video-v2.0", &configured, &request).unwrap();
        assert_eq!(body["width"], 768);
        assert_eq!(body["height"], 1024);
        assert_eq!(body["num_frames"], 81);
    }

    #[test]
    fn v20_keeps_single_image_and_ordered_keyframe_forms_distinct() {
        let mut request = request();
        request.inputs = vec![input("first_frame", b"first"), input("reference", b"middle"), input("last_frame", b"last")];
        let defaults = json!({"image":"stale", "mode":"stale", "extra_body":{"seed":42}});
        let body = build_video_body("agnes-video-v2.0", &defaults, &request).unwrap();
        assert!(body.get("image").is_none());
        assert!(body.get("mode").is_none());
        assert_eq!(body["extra_body"]["mode"], "keyframes");
        assert_eq!(body["extra_body"]["seed"], 42);
        assert_eq!(body["extra_body"]["image"], json!([
            "data:image/png;base64,Zmlyc3Q=", "data:image/png;base64,bWlkZGxl", "data:image/png;base64,bGFzdA==",
        ]));
        request.inputs = vec![input("first_frame", b"first")];
        let body = build_video_body("agnes-video-v2.0", &body, &request).unwrap();
        assert_eq!(body["image"], "data:image/png;base64,Zmlyc3Q=");
        assert!(body["extra_body"].get("image").is_none());
        assert!(body["extra_body"].get("mode").is_none());
        request.inputs = vec![input("last_frame", b"last")];
        assert!(build_video_body("agnes-video-v2.0", &json!({}), &request).is_err());
    }

    #[test]
    fn v20_rejects_invalid_dimensions_frames_and_roles_before_submission() {
        for configured in [json!({"width":0}), json!({"height":"1080"}), json!({"width":1919}),
            json!({"frame_rate":61}), json!({"num_frames":120}), json!({"num_frames":449})] {
            assert!(build_video_body("agnes-video-v2.0", &configured, &request()).is_err());
        }
        for size in ["720P", "0x768", "1112x834", "auto"] {
            let mut request = request();
            request.size = Some(size.into());
            assert!(build_video_body("agnes-video-v2.0", &json!({}), &request).is_err());
        }
        for seconds in [0, 19, u32::MAX] {
            let mut request = request();
            request.seconds = Some(seconds);
            assert!(build_video_body("agnes-video-v2.0", &json!({}), &request).is_err());
        }
        let mut request = request();
        request.inputs = vec![input("reference", b"first"), input("first_frame", b"last")];
        assert!(build_video_body("agnes-video-v2.0", &json!({}), &request).is_err());
    }
}
