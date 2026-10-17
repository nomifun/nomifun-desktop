//! Single-frame capture from a ScreenCast portal's restricted PipeWire remote.
//!
//! This adapter accepts only the fd and node returned by the desktop portal.
//! It never connects to the default PipeWire server or enumerates other nodes.
//! The caller must keep the portal session alive for the duration of the call.

use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use image::RgbaImage;

const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const SAMPLE_WAIT: Duration = Duration::from_millis(100);

/// Check the installed native capture components without opening a desktop
/// session or requesting consent. This does not establish portal permission.
pub fn capture_backend_available() -> bool {
    gst::init().is_ok()
        && ["pipewiresrc", "videoconvert", "videoflip", "appsink"]
            .iter()
            .all(|factory| gst::ElementFactory::find(factory).is_some())
}

/// Capture one frame through the remote explicitly granted by ScreenCast.
///
/// This is a blocking operation; async callers must use a blocking worker. The
/// owned fd stays open until the pipeline is stopped and destroyed. Returned
/// dimensions are physical pixels; the portal's stream size/position describe
/// logical desktop coordinates and are deliberately not used as buffer sizes.
pub fn capture_portal_frame(
    fd: OwnedFd,
    node_id: u32,
    pipewire_serial: Option<u64>,
) -> Result<RgbaImage, String> {
    if pipewire_serial.is_none() && node_id == u32::MAX {
        return Err(capture_error("portal returned the reserved PipeWire any-node ID"));
    }
    gst::init().map_err(|error| {
        format!("COMPUTER_PIPEWIRE_UNAVAILABLE: cannot initialize GStreamer: {error}")
    })?;

    let source = gst::ElementFactory::make("pipewiresrc")
        .property("fd", fd.as_raw_fd())
        .property("num-buffers", 1i32)
        .build()
        .map_err(|error| {
            format!(
                "COMPUTER_PIPEWIRE_UNAVAILABLE: the GStreamer pipewiresrc plugin is required (gstreamer1.0-pipewire on Debian/Ubuntu): {error}"
            )
        })?;
    if let Some(serial) = pipewire_serial {
        // ScreenCast v6 serials are never reused after a node is destroyed.
        // An old plugin must fail rather than silently use a stale node ID.
        if source.find_property("target-object").is_none() {
            return Err(
                "COMPUTER_PIPEWIRE_UNAVAILABLE: installed pipewiresrc cannot target a portal stream by its stable serial".into(),
            );
        }
        source.set_property("target-object", serial.to_string());
    } else {
        // Older portals expose only a node ID. Keep that target restricted to
        // the portal's fd and the lifetime of its still-active session.
        source.set_property("path", node_id.to_string());
    }
    let result = capture_from_source(source, FRAME_TIMEOUT);
    // Explicitly order fd closure after source/pipeline teardown. pipewiresrc
    // duplicates the supplied descriptor for its own connection.
    drop(fd);
    result
}

/// Stop even on errors while building, starting, waiting, or mapping a frame.
struct CapturePipeline {
    pipeline: gst::Pipeline,
    stopped: bool,
}

impl CapturePipeline {
    fn stop(&mut self) -> Result<(), String> {
        self.pipeline.set_state(gst::State::Null).map_err(|error| {
            format!("COMPUTER_PIPEWIRE_CAPTURE_FAILED: cannot stop frame pipeline: {error}")
        })?;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for CapturePipeline {
    fn drop(&mut self) {
        if !self.stopped {
            if let Err(error) = self.stop() {
                tracing::warn!(%error, "PipeWire frame pipeline cleanup failed");
            }
        }
    }
}

fn capture_from_source(source: gst::Element, timeout: Duration) -> Result<RgbaImage, String> {
    let mut capture = CapturePipeline {
        pipeline: gst::Pipeline::new(),
        stopped: false,
    };
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| {
            format!("COMPUTER_PIPEWIRE_UNAVAILABLE: GStreamer videoconvert is required: {error}")
        })?;
    let flip = gst::ElementFactory::make("videoflip")
        .property("video-direction", gst_video::VideoOrientationMethod::Auto)
        .build()
        .map_err(|error| {
            format!("COMPUTER_PIPEWIRE_UNAVAILABLE: GStreamer videoflip is required: {error}")
        })?;
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .build();
    let sink = gst_app::AppSink::builder()
        .caps(&caps)
        .max_buffers(1)
        .drop(true)
        .sync(false)
        // Do not retain a second reference to the PipeWire pool's frame
        // after try_pull_sample has removed it from the appsink queue.
        .property("enable-last-sample", false)
        .build();
    capture
        .pipeline
        .add_many([&source, &convert, &flip, sink.upcast_ref()])
        .map_err(capture_error)?;
    gst::Element::link_many([&source, &convert, &flip, sink.upcast_ref()])
        .map_err(capture_error)?;
    capture
        .pipeline
        .set_state(gst::State::Playing)
        .map_err(capture_error)?;

    let result = wait_for_frame(&capture.pipeline, &sink, timeout);
    let stopped = capture.stop();
    // A frame cannot count as successful capture if its stream failed to stop.
    match (result, stopped) {
        (Ok(image), Ok(())) => Ok(image),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn capture_error(error: impl std::fmt::Display) -> String {
    format!("COMPUTER_PIPEWIRE_CAPTURE_FAILED: {error}")
}

fn wait_for_frame(
    pipeline: &gst::Pipeline,
    sink: &gst_app::AppSink,
    timeout: Duration,
) -> Result<RgbaImage, String> {
    let bus = pipeline
        .bus()
        .ok_or_else(|| capture_error("frame pipeline has no message bus"))?;
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "COMPUTER_PIPEWIRE_FRAME_TIMEOUT: portal stream did not provide a frame in time"
                    .into(),
            );
        }
        let wait = remaining.min(SAMPLE_WAIT);
        if let Some(sample) =
            sink.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos() as u64))
        {
            return rgba_from_sample(&sample);
        }
        for message in bus.iter() {
            if let gst::MessageView::Error(error) = message.view() {
                return Err(capture_error(error.error()));
            }
        }
        if sink.is_eos() {
            return Err(capture_error("portal stream ended before supplying a frame"));
        }
    }
}

fn rgba_from_sample(sample: &gst::Sample) -> Result<RgbaImage, String> {
    let caps = sample
        .caps()
        .ok_or_else(|| capture_error("frame has no negotiated format"))?;
    let info = gst_video::VideoInfo::from_caps(caps).map_err(capture_error)?;
    if info.format() != gst_video::VideoFormat::Rgba {
        return Err(capture_error("frame did not negotiate RGBA pixels"));
    }
    let buffer = sample
        .buffer()
        .ok_or_else(|| capture_error("frame has no pixel buffer"))?;
    if buffer.flags().contains(gst::BufferFlags::CORRUPTED) {
        return Err(capture_error("PipeWire marked the captured frame as corrupted"));
    }
    // VideoFrame honors GstVideoMeta offsets and strides and maps the memory
    // supplied by PipeWire. It also unmaps before the stream is torn down.
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(capture_error)?;
    let stride = frame.plane_stride()[0];
    if stride < 0 {
        return Err(capture_error("frame has an unsupported negative row stride"));
    }
    let plane = frame.plane_data(0).map_err(capture_error)?;
    let image = rgba_from_plane(frame.width(), frame.height(), stride, plane)?;
    if let Some(crop) = buffer.meta::<gst_video::VideoCropMeta>() {
        let (x, y, width, height) = crop.rect();
        // SPA_META_VideoCrop uses a zero width/height for an invalid (unset)
        // crop rectangle. pipewiresrc still attaches that metadata.
        if width != 0 && height != 0 {
            if x.checked_add(width).is_none_or(|right| right > image.width())
                || y.checked_add(height)
                    .is_none_or(|bottom| bottom > image.height())
            {
                return Err(capture_error("frame crop rectangle exceeds its pixel buffer"));
            }
            return Ok(image::imageops::crop_imm(&image, x, y, width, height).to_image());
        }
    }
    Ok(image)
}

fn rgba_from_plane(
    width: u32,
    height: u32,
    stride: i32,
    plane: &[u8],
) -> Result<RgbaImage, String> {
    if width == 0 || height == 0 {
        return Err(capture_error("frame has empty dimensions"));
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| capture_error("frame width overflow"))?;
    // GStreamer's readable plane slice starts at its first row. A negative
    // stride cannot be indexed safely through that slice; fail closed rather
    // than silently treating it as a huge positive stride or flipping pixels.
    let stride = usize::try_from(stride)
        .map_err(|_| capture_error("frame has an unsupported negative row stride"))?;
    if stride < row_bytes {
        return Err(capture_error("frame stride is shorter than one RGBA row"));
    }
    let required = (height as usize - 1)
        .checked_mul(stride)
        .and_then(|offset| offset.checked_add(row_bytes))
        .ok_or_else(|| capture_error("frame size overflow"))?;
    if plane.len() < required {
        return Err(capture_error("frame pixel buffer is truncated"));
    }
    let size = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| capture_error("frame pixel size overflow"))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(size)
        .map_err(|_| capture_error("cannot allocate captured frame"))?;
    for row in 0..height as usize {
        pixels.extend_from_slice(&plane[row * stride..row * stride + row_bytes]);
    }
    RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| capture_error("invalid captured RGBA dimensions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_copy_omits_row_padding_without_reordering_channels() {
        let plane = [1, 2, 3, 4, 99, 99, 99, 99, 5, 6, 7, 8];
        let image = rgba_from_plane(1, 2, 8, &plane).unwrap();
        assert_eq!(image.dimensions(), (1, 2));
        assert_eq!(image.into_raw(), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rgba_copy_rejects_invalid_frame_layouts() {
        for (width, height, stride, plane) in [
            (0, 1, 4, &[0; 4][..]),
            (1, 0, 4, &[0; 4][..]),
            (1, 1, -4, &[0; 4][..]),
            (2, 1, 4, &[0; 8][..]),
            (1, 2, 8, &[0; 11][..]),
        ] {
            assert!(rgba_from_plane(width, height, stride, plane).is_err());
        }
    }

    #[test]
    fn gstreamer_single_frame_converts_real_buffer_to_rgba() {
        gst::init().unwrap();
        // This exercises native GStreamer buffer negotiation and teardown. It
        // is deliberately a synthetic source, not evidence of portal consent
        // or capture of an actual Wayland desktop.
        let source = gst::ElementFactory::make("videotestsrc")
            .property("num-buffers", 1i32)
            .property_from_str("pattern", "red")
            .build()
            .unwrap();
        let image = capture_from_source(source, Duration::from_secs(2)).unwrap();
        assert!(image.width() > 0 && image.height() > 0);
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn sample_respects_video_meta_offset_and_padded_stride() {
        gst::init().unwrap();
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, 1, 2)
            .build()
            .unwrap();
        let mut buffer = gst::Buffer::from_slice(vec![
            99, 99, 99, 99, // Metadata offset, not pixel data.
            1, 2, 3, 4, 99, 99, 99, 99, // Row one plus padding.
            5, 6, 7, 8, 99, 99, 99, 99, // Row two plus padding.
        ]);
        gst_video::VideoMeta::add_full(
            buffer.get_mut().unwrap(),
            gst_video::VideoFrameFlags::empty(),
            gst_video::VideoFormat::Rgba,
            1,
            2,
            &[4],
            &[8],
        )
        .unwrap();
        let sample = gst::Sample::builder()
            .caps(&info.to_caps().unwrap())
            .buffer(&buffer)
            .build();
        assert_eq!(
            rgba_from_sample(&sample).unwrap().into_raw(),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn sample_returns_only_visible_crop_region() {
        gst::init().unwrap();
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, 2, 2)
            .build()
            .unwrap();
        let mut buffer = gst::Buffer::from_slice(vec![
            1, 2, 3, 4, 5, 6, 7, 8, // First row.
            9, 10, 11, 12, 13, 14, 15, 16, // Second row.
        ]);
        gst_video::VideoCropMeta::add(buffer.get_mut().unwrap(), (1, 1, 1, 1));
        let sample = gst::Sample::builder()
            .caps(&info.to_caps().unwrap())
            .buffer(&buffer)
            .build();
        let image = rgba_from_sample(&sample).unwrap();
        assert_eq!(image.dimensions(), (1, 1));
        assert_eq!(image.into_raw(), [13, 14, 15, 16]);
    }

    #[test]
    fn native_pipeline_applies_stream_orientation_before_returning_pixels() {
        gst::init().unwrap();
        let source = gst::parse::bin_from_description(
            r#"videotestsrc num-buffers=1 pattern=red ! video/x-raw,width=2,height=1 ! taginject tags="image-orientation=rotate-90""#,
            true,
        )
        .unwrap();
        let image = capture_from_source(source.upcast(), Duration::from_secs(2)).unwrap();
        assert_eq!(image.dimensions(), (1, 2));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn frame_timeout_stops_the_native_pipeline() {
        gst::init().unwrap();
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, 1, 1)
            .build()
            .unwrap();
        let source = gst_app::AppSrc::builder()
            .caps(&info.to_caps().unwrap())
            .is_live(true)
            .build();
        // A live source that supplies no buffers must return a typed timeout
        // and leave its element stopped, instead of leaking a streaming task.
        let error = capture_from_source(source.clone().upcast(), Duration::from_millis(25))
            .unwrap_err();
        assert!(error.starts_with("COMPUTER_PIPEWIRE_FRAME_TIMEOUT:"));
        assert_eq!(source.current_state(), gst::State::Null);
    }
}
