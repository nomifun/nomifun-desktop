//! Set-of-Marks overlay: draw numbered boxes for each interactable element onto
//! the screenshot the model sees. Self-contained (only the `image` crate) — a
//! tiny embedded 3×5 bitmap digit font renders the `[ref]` labels, so no font
//! asset or extra dependency is needed.
//!
//! Element `bounds` MUST already be in the image's pixel space (the caller
//! converts OS accessibility coordinates → screenshot pixels before calling).

use image::{Rgba, RgbaImage};

use crate::engine::ElementEntry;

/// Distinct, high-contrast mark colors cycled by ref so neighbors differ.
const PALETTE: [[u8; 3]; 6] = [
    [255, 59, 48],  // red
    [0, 122, 255],  // blue
    [52, 199, 89],  // green
    [255, 149, 0],  // orange
    [175, 82, 222], // purple
    [255, 45, 85],  // pink
];

/// 3×5 bitmap font, digits 0-9. Each row's low 3 bits are pixels (MSB = left).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

const SCALE: i64 = 3; // pixels per font cell
const DIGIT_W: i64 = 3 * SCALE;
const DIGIT_H: i64 = 5 * SCALE;
const GAP: i64 = SCALE;
const PAD: i64 = SCALE;

/// Draw a numbered box for each entry.
pub fn draw_set_of_marks(img: &mut RgbaImage, entries: &[ElementEntry]) {
    let (iw, ih) = img.dimensions();
    for e in entries {
        if e.bounds.is_empty() {
            continue;
        }
        let color = PALETTE[(e.r#ref as usize) % PALETTE.len()];
        let x = e.bounds.x.round() as i64;
        let y = e.bounds.y.round() as i64;
        let w = e.bounds.w.round() as i64;
        let h = e.bounds.h.round() as i64;
        draw_rect_border(img, x, y, w, h, color, 2, iw, ih);
        draw_label(img, x, y, e.r#ref, color, iw, ih);
    }
}

fn put(img: &mut RgbaImage, x: i64, y: i64, c: [u8; 3], iw: u32, ih: u32) {
    if x < 0 || y < 0 || x >= iw as i64 || y >= ih as i64 {
        return;
    }
    img.put_pixel(x as u32, y as u32, Rgba([c[0], c[1], c[2], 255]));
}

fn fill_rect(img: &mut RgbaImage, x: i64, y: i64, w: i64, h: i64, c: [u8; 3], iw: u32, ih: u32) {
    for dy in 0..h {
        for dx in 0..w {
            put(img, x + dx, y + dy, c, iw, ih);
        }
    }
}

fn draw_rect_border(
    img: &mut RgbaImage,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    c: [u8; 3],
    t: i64,
    iw: u32,
    ih: u32,
) {
    for k in 0..t {
        // top / bottom
        for dx in 0..w {
            put(img, x + dx, y + k, c, iw, ih);
            put(img, x + dx, y + h - 1 - k, c, iw, ih);
        }
        // left / right
        for dy in 0..h {
            put(img, x + k, y + dy, c, iw, ih);
            put(img, x + w - 1 - k, y + dy, c, iw, ih);
        }
    }
}

fn label_size(n: u32) -> (i64, i64) {
    let digits = n.max(1).to_string().len() as i64;
    let w = PAD * 2 + digits * DIGIT_W + (digits - 1) * GAP;
    let h = PAD * 2 + DIGIT_H;
    (w, h)
}

fn draw_label(img: &mut RgbaImage, ex: i64, ey: i64, n: u32, bg: [u8; 3], iw: u32, ih: u32) {
    let (lw, lh) = label_size(n);
    // Prefer just above the element's top-left; if no room, place inside.
    let lx = ex.max(0);
    let ly = if ey - lh >= 0 { ey - lh } else { ey };
    fill_rect(img, lx, ly, lw, lh, bg, iw, ih);

    let fg = [255u8, 255, 255]; // white digits on the colored chip
    let mut cx = lx + PAD;
    let cy = ly + PAD;
    for ch in n.to_string().chars() {
        let d = ch.to_digit(10).unwrap_or(0) as usize;
        draw_digit(img, cx, cy, DIGITS[d], fg, iw, ih);
        cx += DIGIT_W + GAP;
    }
}

fn draw_digit(img: &mut RgbaImage, x: i64, y: i64, glyph: [u8; 5], c: [u8; 3], iw: u32, ih: u32) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..3i64 {
            // MSB is the leftmost column.
            if bits & (1 << (2 - col)) != 0 {
                fill_rect(
                    img,
                    x + col * SCALE,
                    y + row as i64 * SCALE,
                    SCALE,
                    SCALE,
                    c,
                    iw,
                    ih,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Rect, Source};

    fn entry(r: u32, x: f64, y: f64) -> ElementEntry {
        ElementEntry {
            r#ref: r,
            role: "button".into(),
            name: Some("x".into()),
            value: None,
            states: vec![],
            bounds: Rect { x, y, w: 40.0, h: 20.0 },
            source: Source::A11y,
        }
    }

    #[test]
    fn draws_marks_without_panicking_at_edges() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
        // One in-bounds, one clipped at the top edge (label would go off-screen).
        draw_set_of_marks(&mut img, &[entry(1, 30.0, 40.0), entry(12, 0.0, 0.0)]);
        // Some pixels must now be non-black (a border or label was drawn).
        let changed = img.pixels().any(|p| p[0] > 2 || p[1] > 2 || p[2] > 2);
        assert!(changed);
    }

    #[test]
    fn label_size_grows_with_digits() {
        assert!(label_size(7).0 < label_size(42).0);
        assert!(label_size(42).0 < label_size(123).0);
    }
}
