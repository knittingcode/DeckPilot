//! Built-in vector icons drawn from SVG files of this crate, sharp at every
//! key size: the outline is read once and rasterised at the size it is
//! drawn at, never scaled as a bitmap.
//!
//! The only one is the folder's protected Back key
//! ([`deckpilot_core::device_runtime::symbols::FOLDER_BACK`]):
//! `assets/icons/folder-back.svg`, the one file the renderer (devices and the
//! editor's previews) and the app's own fallback face read. Swapping the
//! icon means replacing that file.
//!
//! The reader is deliberately small and strict, for stroked line icons in
//! the style of Lucide: a `viewBox`, `stroke-width` on the root, and
//! `polyline`, `line` and `path` elements whose paths use `M`, `L`, `H`,
//! `V`, `A` and `Z` (absolute or relative). Strokes have round caps and
//! joins. A file it cannot read draws nothing (a test reads the asset).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; coordinates are clamped before use"
)]

use std::f32::consts::{PI, TAU};
use std::sync::LazyLock;

use ab_glyph_rasterizer::{Point, Rasterizer, point};
use deckpilot_device::Frame;

/// The folder's protected Back key.
pub(crate) const FOLDER_BACK_SVG: &str = include_str!("../assets/icons/folder-back.svg");

static FOLDER_BACK: LazyLock<Option<StrokeIcon>> =
    LazyLock::new(|| StrokeIcon::parse(FOLDER_BACK_SVG));

/// The folder Back icon, if its file could be read.
pub(crate) fn folder_back() -> Option<&'static StrokeIcon> {
    FOLDER_BACK.as_ref()
}

/// A stroked icon: polylines in view box units.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StrokeIcon {
    /// Width and height of the view box (its origin is subtracted).
    size: (f32, f32),
    stroke_width: f32,
    lines: Vec<Vec<(f32, f32)>>,
}

fn attribute<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = element;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.starts_with("=\"") {
            let value = &after[2..];
            return value.find('"').map(|end| &value[..end]);
        }
        rest = after;
    }
}

fn numbers(text: &str) -> Option<Vec<f32>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<f32>| -> Option<()> {
        if !current.is_empty() {
            out.push(current.parse().ok()?);
            current.clear();
        }
        Some(())
    };
    for ch in text.chars() {
        match ch {
            '-' if !current.is_empty() && !current.ends_with('e') => {
                flush(&mut current, &mut out)?;
                current.push(ch);
            }
            '.' if current.contains('.') => {
                flush(&mut current, &mut out)?;
                current.push(ch);
            }
            '0'..='9' | '.' | '-' | 'e' => current.push(ch),
            ch if ch.is_whitespace() || ch == ',' => flush(&mut current, &mut out)?,
            _ => return None,
        }
    }
    flush(&mut current, &mut out)?;
    Some(out)
}

impl StrokeIcon {
    /// Reads `svg`; `None` when it uses anything this reader does not know.
    pub(crate) fn parse(svg: &str) -> Option<Self> {
        let root_end = svg.find('>')?;
        let root = &svg[..root_end];
        if !root.trim_start().starts_with("<svg") {
            return None;
        }
        let view = numbers(attribute(root, "viewBox")?)?;
        let [x0, y0, width, height] = view[..] else {
            return None;
        };
        let stroke_width = attribute(root, "stroke-width").map_or(Some(1.0), |w| w.parse().ok())?;
        let mut lines = Vec::new();
        let mut rest = &svg[root_end + 1..];
        while let Some(start) = rest.find('<') {
            let end = rest[start..].find('>')? + start;
            let element = &rest[start..=end];
            rest = &rest[end + 1..];
            let tag = element[1..]
                .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()?;
            match tag {
                "polyline" => {
                    let values = numbers(attribute(element, "points")?)?;
                    if values.len() % 2 != 0 {
                        return None;
                    }
                    lines.push(values.chunks(2).map(|p| (p[0], p[1])).collect());
                }
                "line" => {
                    let get = |name| attribute(element, name)?.parse::<f32>().ok();
                    lines.push(vec![(get("x1")?, get("y1")?), (get("x2")?, get("y2")?)]);
                }
                "path" => lines.extend(path(attribute(element, "d")?)?),
                "" => {}
                _ if tag.starts_with('/') => {}
                _ => return None,
            }
        }
        for line in &mut lines {
            for p in line.iter_mut() {
                *p = (p.0 - x0, p.1 - y0);
            }
        }
        Some(Self {
            size: (width, height),
            stroke_width,
            lines,
        })
    }

    /// Draws the icon in `color`, its view box fitted into the square at
    /// `(left, top)` of `side` pixels.
    pub(crate) fn draw(&self, frame: &mut Frame, left: f32, top: f32, side: f32, color: [u8; 3]) {
        let scale = side / self.size.0.max(self.size.1);
        let offset = (
            left + (side - self.size.0 * scale) / 2.0,
            top + (side - self.size.1 * scale) / 2.0,
        );
        let radius = self.stroke_width * scale / 2.0;
        let (x0, y0) = ((left - radius).floor() - 1.0, (top - radius).floor() - 1.0);
        let extent = (side + 2.0 * radius).ceil() + 3.0;
        if extent <= 0.0 || extent > 4096.0 {
            return;
        }
        let mut raster = Rasterizer::new(extent as usize, extent as usize);
        let at = |p: (f32, f32)| point(offset.0 + p.0 * scale - x0, offset.1 + p.1 * scale - y0);
        for line in &self.lines {
            let points: Vec<Point> = line.iter().map(|p| at(*p)).collect();
            // Every shape winds the same way, so overlaps add up and are
            // cut at full coverage: the strokes are one shape.
            for pair in points.windows(2) {
                segment(&mut raster, pair[0], pair[1], radius);
            }
            for p in &points {
                disc(&mut raster, *p, radius);
            }
        }
        raster.for_each_pixel_2d(|x, y, coverage| {
            crate::text::blend_at(
                frame,
                x0 + x as f32,
                y0 + y as f32,
                color,
                coverage.min(1.0),
            );
        });
    }
}

/// A stroke's straight piece: a rectangle around it.
fn segment(raster: &mut Rasterizer, a: Point, b: Point, radius: f32) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return;
    }
    let (nx, ny) = (-dy / length * radius, dx / length * radius);
    let corners = [
        point(a.x + nx, a.y + ny),
        point(b.x + nx, b.y + ny),
        point(b.x - nx, b.y - ny),
        point(a.x - nx, a.y - ny),
    ];
    polygon(raster, &corners);
}

/// A round cap or join: a disc, wound like [`segment`]'s rectangles.
fn disc(raster: &mut Rasterizer, center: Point, radius: f32) {
    let steps = 32;
    let corners: Vec<Point> = (0..steps)
        .map(|i| {
            let angle = -TAU * i as f32 / steps as f32;
            point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect();
    polygon(raster, &corners);
}

fn polygon(raster: &mut Rasterizer, corners: &[Point]) {
    for (index, from) in corners.iter().enumerate() {
        raster.draw_line(*from, corners[(index + 1) % corners.len()]);
    }
}

/// The subpaths of path data, arcs flattened.
fn path(data: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    let mut tokens: Vec<(char, Vec<f32>)> = Vec::new();
    let mut command: Option<char> = None;
    let mut args = String::new();
    for ch in data.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' {
            if let Some(c) = command {
                tokens.push((c, numbers(&args)?));
            }
            command = Some(ch);
            args.clear();
        } else {
            args.push(ch);
        }
    }
    if let Some(c) = command {
        tokens.push((c, numbers(&args)?));
    }
    let mut lines: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current = (0.0_f32, 0.0_f32);
    let mut start = current;
    for (command, values) in tokens {
        let relative = command.is_ascii_lowercase();
        let base = |p: (f32, f32), current: (f32, f32)| {
            if relative {
                (current.0 + p.0, current.1 + p.1)
            } else {
                p
            }
        };
        match command.to_ascii_uppercase() {
            'M' => {
                for (index, pair) in values.as_chunks::<2>().0.iter().enumerate() {
                    current = base((pair[0], pair[1]), current);
                    if index == 0 {
                        start = current;
                        lines.push(vec![current]);
                    } else {
                        lines.last_mut()?.push(current);
                    }
                }
            }
            'L' => {
                for pair in values.as_chunks::<2>().0 {
                    current = base((pair[0], pair[1]), current);
                    lines.last_mut()?.push(current);
                }
            }
            'H' => {
                for x in values {
                    current.0 = if relative { current.0 + x } else { x };
                    lines.last_mut()?.push(current);
                }
            }
            'V' => {
                for y in values {
                    current.1 = if relative { current.1 + y } else { y };
                    lines.last_mut()?.push(current);
                }
            }
            'A' => {
                for arc in values.as_chunks::<7>().0 {
                    let to = base((arc[5], arc[6]), current);
                    let points = arc_points(
                        current,
                        to,
                        (arc[0], arc[1]),
                        arc[2],
                        arc[3] != 0.0,
                        arc[4] != 0.0,
                    );
                    lines.last_mut()?.extend(points);
                    current = to;
                }
            }
            'Z' => {
                lines.last_mut()?.push(start);
                current = start;
            }
            _ => return None,
        }
    }
    Some(lines)
}

/// An elliptical arc from `from` to `to` (SVG endpoint parameters), as
/// points after `from`.
fn arc_points(
    from: (f32, f32),
    to: (f32, f32),
    radii: (f32, f32),
    rotation_degrees: f32,
    large: bool,
    sweep: bool,
) -> Vec<(f32, f32)> {
    let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
    if rx <= f32::EPSILON || ry <= f32::EPSILON || from == to {
        return vec![to];
    }
    let phi = rotation_degrees.to_radians();
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut factor = (numerator / denominator).max(0.0).sqrt();
    if large == sweep {
        factor = -factor;
    }
    let (cx1, cy1) = (factor * rx * y1 / ry, -factor * ry * x1 / rx);
    let center = (
        cos * cx1 - sin * cy1 + f32::midpoint(from.0, to.0),
        sin * cx1 + cos * cy1 + f32::midpoint(from.1, to.1),
    );
    let angle = |ux: f32, uy: f32| uy.atan2(ux);
    let theta = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - theta;
    if sweep && delta < 0.0 {
        delta += TAU;
    } else if !sweep && delta > 0.0 {
        delta -= TAU;
    }
    let steps = ((delta.abs() / (PI / 16.0)).ceil() as usize).max(1);
    (1..=steps)
        .map(|i| {
            if i == steps {
                return to;
            }
            let t = theta + delta * i as f32 / steps as f32;
            let (x, y) = (rx * t.cos(), ry * t.sin());
            (cos * x - sin * y + center.0, sin * x + cos * y + center.1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_back_asset_is_read() {
        let icon = folder_back().expect("the folder Back icon is readable");
        assert_eq!(icon.size, (24.0, 24.0));
        assert!((icon.stroke_width - 2.0).abs() < f32::EPSILON);
        // The polyline, and the path with its arc flattened.
        assert_eq!(icon.lines.len(), 2);
        assert_eq!(icon.lines[0], [(14.0, 9.0), (9.0, 4.0), (4.0, 9.0)]);
        let path = &icon.lines[1];
        assert_eq!(path[0], (20.0, 20.0));
        assert_eq!(path[1], (13.0, 20.0));
        assert_eq!(*path.last().unwrap(), (9.0, 4.0));
        // The arc ends at (9, 16), a quarter circle around (13, 16).
        assert!(path.contains(&(9.0, 16.0)));
        for p in &path[2..path.len() - 2] {
            let r = (p.0 - 13.0).hypot(p.1 - 16.0);
            assert!((r - 4.0).abs() < 0.01, "{p:?} is off the arc");
        }
    }

    /// The lit pixels of `frame` (any channel above `level`): their
    /// bounding box, `(left, top, right, bottom)` inclusive.
    fn lit(frame: &Frame, level: u8) -> Option<(u32, u32, u32, u32)> {
        let mut found: Option<(u32, u32, u32, u32)> = None;
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                if frame
                    .pixel(x, y)
                    .is_some_and(|p| p.iter().any(|c| *c > level))
                {
                    let b = found.get_or_insert((x, y, x, y));
                    *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                }
            }
        }
        found
    }

    #[test]
    fn the_protected_back_key_is_its_icon_alone_centred_on_black() {
        let face = deckpilot_core::device_runtime::system_back_face();
        for size in [72_u32, 96, 120] {
            let frame = crate::render(&face, size, size);
            // The golden image: black, and the vector drawn at this very
            // size in the middle. Nothing else (no title, no other mark).
            let mut golden = Frame::filled(size, size, [0, 0, 0]);
            let side = size as f32 * crate::face::MARK_ALONE;
            let corner = (size as f32 - side) / 2.0;
            folder_back()
                .unwrap()
                .draw(&mut golden, corner, corner, side, [255, 255, 255]);
            assert!(frame == golden, "{size} px differs from the vector");
            let (left, top, right, bottom) = lit(&frame, 128).unwrap();
            let center = (size - 1) as f32 / 2.0;
            let mid = |a: u32, b: u32| f32::midpoint(a as f32, b as f32);
            assert!(
                (mid(left, right) - center).abs() <= 1.0,
                "{size}: {left}..{right}"
            );
            assert!(
                (mid(top, bottom) - center).abs() <= 1.0,
                "{size}: {top}..{bottom}"
            );
            // Sharp: full white inside the strokes, at every size.
            assert!(
                (0..size)
                    .flat_map(|y| (0..size).map(move |x| (x, y)))
                    .any(|(x, y)| frame.pixel(x, y) == Some([255, 255, 255]))
            );
        }
        // A key titled "Back" (a layout's own Back) is not drawn like it.
        let titled = deckpilot_core::KeyFace::new(deckpilot_core::FaceStyle::Back, "Back")
            .with_symbol(Some(deckpilot_core::device_runtime::symbols::BACK));
        assert!(
            crate::render(&titled, 72, 72) != crate::render(&face, 72, 72),
            "a titled Back key looks like the protected one"
        );
    }

    #[test]
    fn unknown_shapes_are_refused() {
        assert_eq!(
            StrokeIcon::parse(r#"<svg viewBox="0 0 24 24"><rect x="1"/></svg>"#),
            None
        );
        assert_eq!(
            StrokeIcon::parse(r#"<svg viewBox="0 0 24 24"><path d="M1 1Q2 2 3 3"/></svg>"#),
            None
        );
        assert!(
            StrokeIcon::parse(r#"<svg viewBox="0 0 24 24" stroke-width="2"><line x1="1" y1="2" x2="3" y2="4"/></svg>"#)
                .is_some()
        );
    }
}
