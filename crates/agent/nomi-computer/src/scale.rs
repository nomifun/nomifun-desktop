//! Pure geometry helpers for screenshot downscaling and coordinate mapping.
//!
//! The LLM sees a (possibly downscaled) screenshot and replies with pixel
//! coordinates in that image. Input synthesis needs logical screen
//! coordinates (on macOS Retina xcap captures physical pixels while enigo
//! expects logical points), so every pointer action goes through
//! `map_llm_coord` and cursor reporting through `map_screen_coord`.

/// Compute downscaled dimensions so the longest edge fits `max_edge`.
/// Returns the original size when it already fits, or when `max_edge` is 0
/// (treated as "no limit"). Never returns a zero dimension.
pub fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    if max_edge == 0 || (width <= max_edge && height <= max_edge) {
        return (width, height);
    }
    let longest = width.max(height) as f64;
    let scale = max_edge as f64 / longest;
    let w = ((width as f64 * scale).round() as u32).max(1);
    let h = ((height as f64 * scale).round() as u32).max(1);
    (w, h)
}

/// Map a coordinate in screenshot pixel space to logical screen space.
///
/// Uses pixel-center mapping so the round trip with `map_screen_coord` is
/// exact whenever the logical size is >= the image size. The result is
/// clamped into the screen bounds so out-of-range model output cannot click
/// outside the display.
pub fn map_llm_coord(
    llm_x: i32,
    llm_y: i32,
    img_w: u32,
    img_h: u32,
    logical_w: u32,
    logical_h: u32,
) -> (i32, i32) {
    if img_w == 0 || img_h == 0 || logical_w == 0 || logical_h == 0 {
        return (llm_x, llm_y);
    }
    let x = (f64::from(llm_x) + 0.5) * f64::from(logical_w) / f64::from(img_w);
    let y = (f64::from(llm_y) + 0.5) * f64::from(logical_h) / f64::from(img_h);
    (
        (x.floor() as i32).clamp(0, logical_w as i32 - 1),
        (y.floor() as i32).clamp(0, logical_h as i32 - 1),
    )
}

/// Inverse of `map_llm_coord`: map a logical screen coordinate back to
/// screenshot pixel space (e.g. to report the cursor position in the
/// coordinate system the model is working in).
pub fn map_screen_coord(
    screen_x: i32,
    screen_y: i32,
    img_w: u32,
    img_h: u32,
    logical_w: u32,
    logical_h: u32,
) -> (i32, i32) {
    if img_w == 0 || img_h == 0 || logical_w == 0 || logical_h == 0 {
        return (screen_x, screen_y);
    }
    let x = (f64::from(screen_x) + 0.5) * f64::from(img_w) / f64::from(logical_w);
    let y = (f64::from(screen_y) + 0.5) * f64::from(img_h) / f64::from(logical_h);
    (
        (x.floor() as i32).clamp(0, img_w as i32 - 1),
        (y.floor() as i32).clamp(0, img_h as i32 - 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- fit_within ---

    #[test]
    fn fit_within_already_fits() {
        assert_eq!(fit_within(800, 600, 1568), (800, 600));
    }

    #[test]
    fn fit_within_exactly_at_limit() {
        assert_eq!(fit_within(1568, 980, 1568), (1568, 980));
    }

    #[test]
    fn fit_within_landscape_downscale() {
        // Retina MacBook: 2880x1800 physical -> longest edge 1568
        assert_eq!(fit_within(2880, 1800, 1568), (1568, 980));
    }

    #[test]
    fn fit_within_portrait_downscale() {
        assert_eq!(fit_within(1800, 2880, 1568), (980, 1568));
    }

    #[test]
    fn fit_within_square() {
        assert_eq!(fit_within(2000, 2000, 1000), (1000, 1000));
    }

    #[test]
    fn fit_within_extreme_ratio_never_zero() {
        // 10000:1 aspect ratio must not collapse the short edge to 0
        assert_eq!(fit_within(10000, 1, 100), (100, 1));
        assert_eq!(fit_within(1, 10000, 100), (1, 100));
    }

    #[test]
    fn fit_within_zero_max_edge_means_no_limit() {
        assert_eq!(fit_within(2880, 1800, 0), (2880, 1800));
    }

    #[test]
    fn fit_within_longest_edge_is_exact() {
        let (w, h) = fit_within(2879, 1799, 1568);
        assert_eq!(w.max(h), 1568);
        assert!(w >= 1 && h >= 1);
    }

    // --- map_llm_coord ---

    #[test]
    fn map_llm_identity_when_same_size() {
        assert_eq!(map_llm_coord(10, 20, 1440, 900, 1440, 900), (10, 20));
        assert_eq!(map_llm_coord(0, 0, 1440, 900, 1440, 900), (0, 0));
        assert_eq!(map_llm_coord(1439, 899, 1440, 900, 1440, 900), (1439, 899));
    }

    #[test]
    fn map_llm_corners_map_to_screen_corners() {
        // Screenshot 1568x980 of a 1440x900 logical screen (Retina capture
        // downscaled, but still larger than logical).
        assert_eq!(map_llm_coord(0, 0, 1568, 980, 1440, 900), (0, 0));
        assert_eq!(map_llm_coord(1567, 979, 1568, 980, 1440, 900), (1439, 899));
    }

    #[test]
    fn map_llm_center_maps_to_center() {
        let (x, y) = map_llm_coord(784, 490, 1568, 980, 1440, 900);
        assert!((x - 720).abs() <= 1, "x = {x}");
        assert!((y - 450).abs() <= 1, "y = {y}");
    }

    #[test]
    fn map_llm_clamps_out_of_range() {
        assert_eq!(map_llm_coord(-50, -50, 1000, 800, 1440, 900), (0, 0));
        assert_eq!(
            map_llm_coord(99999, 99999, 1000, 800, 1440, 900),
            (1439, 899)
        );
    }

    #[test]
    fn map_llm_zero_dims_passthrough() {
        assert_eq!(map_llm_coord(5, 7, 0, 0, 1440, 900), (5, 7));
        assert_eq!(map_llm_coord(5, 7, 1000, 800, 0, 0), (5, 7));
    }

    // --- round trips ---

    #[test]
    fn round_trip_exact_when_screen_larger_than_image() {
        // image 1000 wide, logical 1440 wide: llm -> screen -> llm is identity
        for k in 0..1000 {
            let (sx, _) = map_llm_coord(k, 0, 1000, 800, 1440, 900);
            let (back, _) = map_screen_coord(sx, 0, 1000, 800, 1440, 900);
            assert_eq!(back, k, "round trip failed for {k}");
        }
    }

    #[test]
    fn round_trip_within_one_pixel_when_image_larger_than_screen() {
        // image 1568 wide, logical 1440 wide: contraction loses at most 1px
        for k in (0..1568).step_by(7) {
            let (sx, _) = map_llm_coord(k, 0, 1568, 980, 1440, 900);
            let (back, _) = map_screen_coord(sx, 0, 1568, 980, 1440, 900);
            assert!((back - k).abs() <= 1, "round trip drifted for {k}: {back}");
        }
    }

    #[test]
    fn screen_round_trip_exact_when_image_larger_than_screen() {
        // screen -> llm -> screen is identity in the contractive direction
        for k in 0..1440 {
            let (ix, _) = map_screen_coord(k, 0, 1568, 980, 1440, 900);
            let (back, _) = map_llm_coord(ix, 0, 1568, 980, 1440, 900);
            assert_eq!(back, k, "screen round trip failed for {k}");
        }
    }
}
