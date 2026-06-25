//! Windows OCR via `Windows.Media.Ocr` — on-device, with CJK support (essential
//! for a Chinese-first product). Mirrors the macOS Vision backend: the
//! screenshot is encoded to PNG, decoded into a `SoftwareBitmap` via
//! `BitmapDecoder` (so we avoid hand-building a pixel buffer), then recognized.
//! `OcrEngine`/`OcrResult` report word bounding boxes already in pixel space
//! with a top-left origin — the same space the overlay/tool use — so no flip is
//! needed.
//!
//! WinRT activation requires COM to be initialized on the calling thread. The
//! tool calls this from `spawn_blocking`, whose pooled threads are not
//! necessarily initialized, so we initialize COM (idempotently) up front.

use std::cell::Cell;
use std::io::Cursor;

use windows::Globalization::Language;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap,
};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

use crate::engine::{A11yError, OcrLine, Rect};

fn win_err(ctx: &str, e: windows::core::Error) -> A11yError {
    A11yError::Backend(format!("OCR: {ctx}: {e}"))
}

thread_local! {
    /// Whether this thread has already initialized COM. WinRT activation
    /// (OcrEngine, BitmapDecoder) requires COM on the calling thread; the tool
    /// calls us from pooled `spawn_blocking` threads. Initialize at most ONCE
    /// per thread and intentionally never `CoUninitialize` — these are
    /// process-lifetime pool threads, so a single MTA init is correct and a
    /// per-call init/leak is avoided.
    static COM_READY: Cell<bool> = const { Cell::new(false) };
}

fn ensure_com() {
    COM_READY.with(|ready| {
        if !ready.get() {
            // Ignore S_FALSE / RPC_E_CHANGED_MODE — the thread is usable either way.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            ready.set(true);
        }
    });
}

/// Build an OCR engine: prefer the caller's requested languages (e.g.
/// `zh-Hans`) when an OCR pack for them is installed, else fall back to the
/// user-profile languages.
fn make_engine(langs: &[String]) -> Result<OcrEngine, A11yError> {
    for l in langs {
        if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(l.as_str())) {
            if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Ok(engine);
                }
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
        A11yError::Backend(format!(
            "OCR engine unavailable: {e}. Install an OCR language pack (Settings → Time & \
             Language → Language → add a language and enable its Optical character recognition \
             feature)."
        ))
    })
}

pub fn ocr_screenshot(img: &image::RgbaImage, langs: &[String]) -> Result<Vec<OcrLine>, A11yError> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }

    // WinRT activation needs COM on this thread (initialized at most once).
    ensure_com();

    // Encode to PNG so BitmapDecoder can decode it into a SoftwareBitmap.
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| A11yError::Backend(format!("OCR: PNG encode failed: {e}")))?;

    let engine = make_engine(langs)?;

    // OcrEngine rejects images larger than MaxImageDimension on a side. The tool
    // already downscales screenshots well under this, but guard defensively with
    // a clear error rather than letting RecognizeAsync fail opaquely.
    if let Ok(max) = OcrEngine::MaxImageDimension() {
        if w > max || h > max {
            return Err(A11yError::Backend(format!(
                "OCR: image {w}x{h} exceeds the engine's max dimension {max} per side; downscale \
                 before OCR"
            )));
        }
    }

    // PNG bytes → in-memory stream → SoftwareBitmap.
    let stream = InMemoryRandomAccessStream::new().map_err(|e| win_err("create stream", e))?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| win_err("create writer", e))?;
    writer.WriteBytes(&png).map_err(|e| win_err("write bytes", e))?;
    writer
        .StoreAsync()
        .map_err(|e| win_err("store", e))?
        .get()
        .map_err(|e| win_err("store.get", e))?;
    writer
        .FlushAsync()
        .map_err(|e| win_err("flush", e))?
        .get()
        .map_err(|e| win_err("flush.get", e))?;
    writer
        .DetachStream()
        .map_err(|e| win_err("detach stream", e))?;
    stream.Seek(0).map_err(|e| win_err("seek", e))?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| win_err("create decoder", e))?
        .get()
        .map_err(|e| win_err("decoder.get", e))?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| win_err("get bitmap", e))?
        .get()
        .map_err(|e| win_err("bitmap.get", e))?;
    // The PNG decoder auto-selects the pixel format (often Rgba8); OcrEngine
    // reliably accepts Bgra8/Premultiplied, so normalize before recognition
    // instead of relying on an undocumented accepted-format set.
    let bitmap = SoftwareBitmap::ConvertWithAlpha(
        &bitmap,
        BitmapPixelFormat::Bgra8,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(|e| win_err("convert to bgra8", e))?;

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| win_err("recognize", e))?
        .get()
        .map_err(|e| win_err("recognize.get", e))?;

    let mut out = Vec::new();
    let lines = result.Lines().map_err(|e| win_err("lines", e))?;
    for line in lines {
        let text = line.Text().map_err(|e| win_err("line text", e))?.to_string();
        if text.trim().is_empty() {
            continue;
        }
        // Union of the line's word bounding rects (already pixel, top-left).
        let words = line.Words().map_err(|e| win_err("words", e))?;
        let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
        let mut any = false;
        for word in words {
            let r = word.BoundingRect().map_err(|e| win_err("word rect", e))?;
            min_x = min_x.min(r.X as f64);
            min_y = min_y.min(r.Y as f64);
            max_x = max_x.max((r.X + r.Width) as f64);
            max_y = max_y.max((r.Y + r.Height) as f64);
            any = true;
        }
        let bounds = if any && max_x >= min_x && max_y >= min_y {
            Rect {
                x: min_x,
                y: min_y,
                w: max_x - min_x,
                h: max_y - min_y,
            }
        } else {
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            }
        };
        out.push(OcrLine { text, bounds });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_blank_image_runs_without_error() {
        // Exercises the full Windows.Media.Ocr FFI path (compile + link + run).
        // A blank image yields no recognized text; OCR needs no permission, but
        // does require an OCR language pack (English ships by default on
        // Windows 10/11).
        let img = image::RgbaImage::from_pixel(80, 40, image::Rgba([255, 255, 255, 255]));
        match ocr_screenshot(&img, &["en-US".to_string()]) {
            Ok(lines) => assert!(lines.iter().all(|l| !l.text.trim().is_empty())),
            Err(A11yError::Backend(msg)) if msg.contains("language pack") => {
                eprintln!("skipping: no OCR language pack installed ({msg})");
            }
            Err(e) => panic!("OCR failed unexpectedly: {e}"),
        }
    }

    #[test]
    fn ocr_empty_image_is_empty() {
        assert!(ocr_screenshot(&image::RgbaImage::new(0, 0), &[]).unwrap().is_empty());
    }
}
