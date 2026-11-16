//! The marks of a key's live state beyond texts, the indicator, progress
//! and the toggle: an accent background, a glyph, a badge and a series.
//!
//! Everything here is drawn from a typed description at the key's own
//! resolution: colours come from a fixed palette, glyphs are a closed set
//! of shapes defined below, the badge uses the built-in bitmap font. No
//! image, font or markup from a state provider is ever decoded.

use deckpilot_core::{Accent, Glyph};
use deckpilot_device::Frame;
use font8x8::legacy::BASIC_LEGACY;

/// The background of an accent: dark enough for a light title.
pub(crate) fn accent_color(accent: Accent) -> [u8; 3] {
    match accent {
        Accent::Blue => [30, 64, 175],
        Accent::Green => [22, 101, 52],
        Accent::Amber => [146, 64, 14],
        Accent::Red => [153, 27, 27],
        Accent::Purple => [91, 33, 182],
        Accent::Teal => [17, 94, 89],
        Accent::Grey => [63, 63, 70],
    }
}

/// `color` moved towards white by `percent`.
pub(crate) fn lighten(color: [u8; 3], percent: u16) -> [u8; 3] {
    color.map(|c| {
        let c = u16::from(c);
        u8::try_from(c + (255 - c) * percent.min(100) / 100).unwrap_or(u8::MAX)
    })
}

type Point = (f32, f32);

/// Even-odd test of `(u, v)` against a polygon.
fn poly(points: &[Point], u: f32, v: f32) -> bool {
    let mut inside = false;
    let mut j = points.len().wrapping_sub(1);
    for (i, &(xi, yi)) in points.iter().enumerate() {
        let (xj, yj) = points[j];
        if (yi > v) != (yj > v) && u < (xj - xi) * (v - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn disc((cx, cy): Point, radius: f32, u: f32, v: f32) -> bool {
    (u - cx).powi(2) + (v - cy).powi(2) <= radius * radius
}

fn ring(center: Point, radius: f32, thickness: f32, u: f32, v: f32) -> bool {
    disc(center, radius, u, v) && !disc(center, radius - thickness, u, v)
}

fn rect((x0, y0): Point, (x1, y1): Point, u: f32, v: f32) -> bool {
    (x0..=x1).contains(&u) && (y0..=y1).contains(&v)
}

/// Whether `(u, v)` is within `half` of the segment from `a` to `b`.
fn stroke(from: Point, to: Point, half: f32, u: f32, v: f32) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx * dx + dy * dy;
    let along = if length == 0.0 {
        0.0
    } else {
        (((u - from.0) * dx + (v - from.1) * dy) / length).clamp(0.0, 1.0)
    };
    disc((from.0 + along * dx, from.1 + along * dy), half, u, v)
}

const ARROW: [Point; 7] = [
    (0.5, 0.12),
    (0.88, 0.58),
    (0.62, 0.58),
    (0.62, 0.88),
    (0.38, 0.88),
    (0.38, 0.58),
    (0.12, 0.58),
];
const STAR: [Point; 10] = [
    (0.5, 0.07),
    (0.612, 0.366),
    (0.928, 0.381),
    (0.681, 0.579),
    (0.765, 0.884),
    (0.5, 0.71),
    (0.235, 0.884),
    (0.319, 0.579),
    (0.072, 0.381),
    (0.388, 0.366),
];
const BELL: [Point; 7] = [
    (0.5, 0.1),
    (0.7, 0.22),
    (0.76, 0.6),
    (0.88, 0.74),
    (0.12, 0.74),
    (0.24, 0.6),
    (0.3, 0.22),
];

/// Whether the point `(u, v)` of the unit square (y down) belongs to
/// `glyph`.
fn covers(glyph: Glyph, u: f32, v: f32) -> bool {
    match glyph {
        Glyph::Check => {
            stroke((0.16, 0.54), (0.4, 0.78), 0.08, u, v)
                || stroke((0.4, 0.78), (0.86, 0.24), 0.08, u, v)
        }
        Glyph::Cross => {
            stroke((0.2, 0.2), (0.8, 0.8), 0.08, u, v) || stroke((0.8, 0.2), (0.2, 0.8), 0.08, u, v)
        }
        Glyph::Play => poly(&[(0.24, 0.14), (0.86, 0.5), (0.24, 0.86)], u, v),
        Glyph::Pause => {
            rect((0.22, 0.16), (0.42, 0.84), u, v) || rect((0.58, 0.16), (0.78, 0.84), u, v)
        }
        Glyph::Stop => rect((0.2, 0.2), (0.8, 0.8), u, v),
        Glyph::Timer => {
            ring((0.5, 0.56), 0.36, 0.09, u, v)
                || rect((0.42, 0.06), (0.58, 0.16), u, v)
                || stroke((0.5, 0.56), (0.5, 0.36), 0.045, u, v)
                || stroke((0.5, 0.56), (0.64, 0.64), 0.045, u, v)
        }
        Glyph::Bell => poly(&BELL, u, v) || disc((0.5, 0.84), 0.08, u, v),
        Glyph::Up => poly(&ARROW, u, v),
        Glyph::Down => poly(&ARROW, u, 1.0 - v),
        Glyph::Dot => disc((0.5, 0.5), 0.22, u, v),
        Glyph::Circle => ring((0.5, 0.5), 0.4, 0.1, u, v),
        Glyph::Square => {
            rect((0.16, 0.16), (0.84, 0.84), u, v) && !rect((0.27, 0.27), (0.73, 0.73), u, v)
        }
        Glyph::Diamond => poly(&[(0.5, 0.08), (0.92, 0.5), (0.5, 0.92), (0.08, 0.5)], u, v),
        Glyph::Triangle => poly(&[(0.5, 0.12), (0.92, 0.86), (0.08, 0.86)], u, v),
        Glyph::Star => poly(&STAR, u, v),
        Glyph::Heart => {
            disc((0.31, 0.34), 0.21, u, v)
                || disc((0.69, 0.34), 0.21, u, v)
                || poly(&[(0.12, 0.44), (0.88, 0.44), (0.5, 0.9)], u, v)
        }
    }
}

/// Where a glyph goes on a key: a square in the upper part, `(x, y, side)`.
fn glyph_box(width: u32, height: u32) -> (u32, u32, u32) {
    let side = (width.min(height) * 2 / 5).max(6);
    (width.saturating_sub(side) / 2, (height / 8).max(2), side)
}

/// Draws `glyph` in the upper part of the key.
pub(crate) fn draw_glyph(frame: &mut Frame, glyph: Glyph, color: [u8; 3]) {
    let (x0, y0, side) = glyph_box(frame.width(), frame.height());
    #[expect(
        clippy::cast_precision_loss,
        reason = "a key is a few hundred pixels wide at most"
    )]
    let unit = |pixel: u32| (pixel as f32 + 0.5) / side as f32;
    for dy in 0..side {
        for dx in 0..side {
            if covers(glyph, unit(dx), unit(dy)) {
                frame.put(x0 + dx, y0 + dy, color);
            }
        }
    }
}

/// Draws `series` (values 0–100, oldest first) as a line across the lower
/// half of the key, above where the progress bar goes. Fewer than two
/// points are no chart.
pub(crate) fn draw_series(frame: &mut Frame, series: &[u8], color: [u8; 3]) {
    let (width, height) = (frame.width(), frame.height());
    if series.len() < 2 || width < 8 || height < 8 {
        return;
    }
    let (left, right) = (2, width - 3);
    let (top, bottom) = (height * 9 / 20, height - (height / 8).max(3));
    let span = u64::from(bottom - top);
    let level =
        |value: u8| bottom - u32::try_from(span * u64::from(value.min(100)) / 100).unwrap_or(0);
    let last = u64::try_from(series.len() - 1).unwrap_or(1);
    let thickness = (height / 40).max(1);
    let mut previous: Option<u32> = None;
    for x in left..=right {
        // The position between two points, in 1/1024 of a step.
        let at = u64::from(x - left) * last * 1024 / u64::from(right - left);
        let index = usize::try_from(at / 1024)
            .unwrap_or(0)
            .min(series.len() - 1);
        let next = (index + 1).min(series.len() - 1);
        let part = at % 1024;
        let here = u64::from(level(series[index]));
        let there = u64::from(level(series[next]));
        let y = u32::try_from((here * (1024 - part) + there * part) / 1024).unwrap_or(bottom);
        // Join with the previous column so steep steps stay one line.
        let (from, to) = previous.map_or((y, y), |p| (p.min(y), p.max(y)));
        for line in from..=to {
            for dy in 0..thickness {
                frame.put(x, line + dy, color);
            }
        }
        previous = Some(y);
    }
}

/// Draws `badge` (at most four characters of the built-in font) on a dark
/// plate in the top left corner, `below` pixels from the top edge.
pub(crate) fn draw_badge(frame: &mut Frame, badge: &str, below: u32, text: [u8; 3]) {
    const CELL: u32 = 8;
    const PLATE: [u8; 3] = [9, 9, 11];
    let count = u32::try_from(badge.chars().take(4).count()).unwrap_or(0);
    if count == 0 {
        return;
    }
    let scale = if frame.width() >= 120 { 2 } else { 1 };
    let (x0, y0) = (2, below + 2);
    let (plate_w, plate_h) = (count * CELL * scale + 2, CELL * scale + 2);
    for dy in 0..plate_h {
        for dx in 0..plate_w {
            frame.put(x0 + dx, y0 + dy, PLATE);
        }
    }
    for (index, ch) in badge.chars().take(4).enumerate() {
        let Some(rows) = BASIC_LEGACY
            .get(ch as usize)
            .or(BASIC_LEGACY.get(usize::from(b'?')))
        else {
            continue;
        };
        let x = x0 + 1 + u32::try_from(index).unwrap_or(0) * CELL * scale;
        for (row, bits) in rows.iter().enumerate() {
            let row = u32::try_from(row).unwrap_or(0);
            for column in 0..CELL {
                if bits & (1 << column) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        frame.put(x + column * scale + dx, y0 + 1 + row * scale + dy, text);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(frame: &Frame, color: [u8; 3]) -> usize {
        (0..frame.height())
            .flat_map(|y| (0..frame.width()).map(move |x| (x, y)))
            .filter(|(x, y)| frame.pixel(*x, *y) == Some(color))
            .count()
    }

    const INK: [u8; 3] = [240, 240, 240];

    #[test]
    fn every_glyph_is_a_distinct_shape_inside_its_box_at_any_size() {
        for (w, h) in [(72, 72), (96, 96), (144, 144), (200, 100), (16, 16), (1, 1)] {
            let mut seen = std::collections::BTreeSet::new();
            for glyph in Glyph::ALL {
                let mut frame = Frame::filled(w, h, [0, 0, 0]);
                draw_glyph(&mut frame, glyph, INK);
                let lit = count(&frame, INK);
                if w < 16 {
                    continue; // drawn without a panic is all a 1 px key needs
                }
                let (x0, y0, side) = glyph_box(w, h);
                let area = usize::try_from(side * side).unwrap();
                assert!(lit > area / 12 && lit < area, "{glyph:?} at {w}x{h}: {lit}");
                // Nothing outside the box.
                for y in 0..h {
                    for x in 0..w {
                        let inside = (x0..x0 + side).contains(&x) && (y0..y0 + side).contains(&y);
                        assert!(inside || frame.pixel(x, y) == Some([0, 0, 0]));
                    }
                }
                // On the smallest keys a shape is a few pixels and two may
                // coincide; from 72 px on every one is told apart.
                assert!(
                    w < 72 || seen.insert(frame.rgb().to_vec()),
                    "{glyph:?} repeats a shape"
                );
            }
        }
    }

    #[test]
    fn a_series_is_one_line_that_follows_its_values() {
        let mut frame = Frame::filled(96, 96, [0, 0, 0]);
        draw_series(&mut frame, &[0, 100], INK);
        let row_of = |frame: &Frame, x: u32| {
            (0..frame.height())
                .find(|y| frame.pixel(x, *y) == Some(INK))
                .unwrap_or_else(|| panic!("column {x} has no point"))
        };
        // Every column between the margins has a point, rising to the right.
        let rows: Vec<u32> = (2..=93).map(|x| row_of(&frame, x)).collect();
        assert!(rows.windows(2).all(|pair| pair[1] <= pair[0]), "{rows:?}");
        assert!(rows[0] > rows[91] + 20);
        // Within the lower half, clear of the bottom edge and the margins.
        assert!(rows.iter().all(|y| (43..=84).contains(y)), "{rows:?}");
        assert_eq!(frame.pixel(0, rows[0]), Some([0, 0, 0]));

        // Values above 100 are drawn as 100; one point or a tiny key: nothing.
        let mut capped = Frame::filled(96, 96, [0, 0, 0]);
        draw_series(&mut capped, &[0, 250], INK);
        assert_eq!(capped.rgb(), frame.rgb());
        for (series, size) in [(&[50u8][..], 96), (&[][..], 96), (&[0, 100][..], 4)] {
            let mut blank = Frame::filled(size, size, [0, 0, 0]);
            draw_series(&mut blank, series, INK);
            assert_eq!(count(&blank, INK), 0);
        }
        // The longest series on the smallest key with a display.
        let mut small = Frame::filled(8, 8, [0, 0, 0]);
        draw_series(&mut small, &[100; 32], INK);
        assert!(count(&small, INK) > 0);
    }

    #[test]
    fn a_badge_is_a_plate_in_the_top_left_corner() {
        let mut frame = Frame::filled(72, 72, [50, 50, 50]);
        draw_badge(&mut frame, "12", 0, INK);
        assert!(count(&frame, INK) > 10);
        // Two cells wide, in the corner, nothing elsewhere.
        for y in 0..72 {
            for x in 0..72 {
                let plate = (2..20).contains(&x) && (2..12).contains(&y);
                assert!(plate || frame.pixel(x, y) == Some([50, 50, 50]), "{x},{y}");
            }
        }
        // More than four characters are not drawn; characters outside the
        // font are `?`; an empty badge is nothing.
        let mut long = Frame::filled(72, 72, [50, 50, 50]);
        draw_badge(&mut long, "123456789", 0, INK);
        assert_eq!(long.pixel(40, 6), Some([50, 50, 50]));
        let (mut odd, mut question) = (frame.clone(), frame.clone());
        draw_badge(&mut odd, "\u{1F600}", 0, INK);
        draw_badge(&mut question, "?", 0, INK);
        assert_eq!(odd.rgb(), question.rgb());
        let mut empty = Frame::filled(72, 72, [50, 50, 50]);
        draw_badge(&mut empty, "", 0, INK);
        assert_eq!(count(&empty, [50, 50, 50]), 72 * 72);
        // A key too small for it: clipped, no panic.
        draw_badge(&mut Frame::filled(4, 4, [0, 0, 0]), "9999", 3, INK);
    }

    #[test]
    fn accents_are_dark_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for accent in Accent::ALL {
            let [r, g, b] = accent_color(accent);
            // Relative luminance well below a light title's.
            let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
            assert!(luma < 90, "{accent:?}: {luma}");
            assert!(seen.insert([r, g, b]));
        }
        assert_eq!(lighten([0, 100, 255], 50), [127, 177, 255]);
        assert_eq!(lighten([10, 10, 10], 250), [255, 255, 255]);
    }
}
