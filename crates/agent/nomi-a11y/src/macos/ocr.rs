//! macOS OCR via Vision.framework (`VNRecognizeTextRequest`) — on-device, with
//! CJK support (essential for a Chinese-first product). The screenshot is fed
//! in as PNG data (`VNImageRequestHandler initWithData:`) so we avoid
//! hand-building a CGImage. Vision returns normalized bounding boxes with a
//! bottom-left origin; we convert them to pixel rectangles with a top-left
//! origin to match the screenshot space the overlay/tool use.
//!
//! OCR has no main-thread/run-loop affinity, so this runs on whatever thread
//! the caller uses (the tool calls it from `spawn_blocking`).

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};

use crate::engine::{A11yError, OcrLine, Rect};

pub fn ocr_screenshot(img: &image::RgbaImage, langs: &[String]) -> Result<Vec<OcrLine>, A11yError> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }

    // Encode to PNG so VNImageRequestHandler can decode it directly.
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| A11yError::Backend(format!("OCR: PNG encode failed: {e}")))?;

    unsafe {
        let data = NSData::with_bytes(&png);

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        if !langs.is_empty() {
            let ns: Vec<Retained<NSString>> =
                langs.iter().map(|l| NSString::from_str(l)).collect();
            let arr = NSArray::from_retained_slice(&ns);
            request.setRecognitionLanguages(&arr);
        }

        let options: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &options,
        );

        let req_ref: &VNRequest = &request;
        let requests = NSArray::from_slice(&[req_ref]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| A11yError::Backend(format!("OCR perform failed: {e:?}")))?;

        let Some(results) = request.results() else {
            return Ok(Vec::new());
        };

        let mut lines = Vec::new();
        for obs in results.iter() {
            let top = obs.topCandidates(1);
            let Some(text) = top.firstObject() else {
                continue;
            };
            let s = text.string().to_string();
            if s.trim().is_empty() {
                continue;
            }
            // Normalized (0..1), bottom-left origin → pixel, top-left origin.
            let bb = obs.boundingBox();
            let px = bb.origin.x * w as f64;
            let pw = bb.size.width * w as f64;
            let ph = bb.size.height * h as f64;
            let py = (1.0 - bb.origin.y - bb.size.height) * h as f64;
            lines.push(OcrLine {
                text: s,
                bounds: Rect {
                    x: px,
                    y: py,
                    w: pw,
                    h: ph,
                },
            });
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_blank_image_runs_without_error() {
        // Exercises the full Vision FFI path (compile + link + run). A blank
        // image yields no recognized text; OCR needs no TCC permission.
        let img = image::RgbaImage::from_pixel(80, 40, image::Rgba([255, 255, 255, 255]));
        let lines = ocr_screenshot(&img, &["en-US".to_string()]).expect("ocr should not error");
        assert!(lines.iter().all(|l| !l.text.trim().is_empty()));
    }

    #[test]
    fn ocr_empty_image_is_empty() {
        let img = image::RgbaImage::new(0, 0);
        assert!(ocr_screenshot(&img, &[]).unwrap().is_empty());
    }
}

