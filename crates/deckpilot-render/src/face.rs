//! Renders [`KeyFace`]s into device-independent [`Frame`]s.
//!
//! The renderer only needs the target size, so the same code serves any key
//! or display size: every measure below is a share of the key's shorter
//! side, so a 72 px key (Stream Deck MK.2), a 96 px key (XL), a 120 px key
//! (Stream Deck +) and a 200 x 100 touch-strip zone are laid out alike.
//!
//! A key is **icon first**: its main mark sits in the upper part, the title
//! (Inter Medium) below it and a value line (the face's detail, Inter
//! semibold, e.g. a monitor's `23%`) under the title. The mark is, in this
//! order, the binding's own icon (an image the user imported, drawn under
//! everything as before), the state's glyph (a closed set of shapes,
//! [`crate::marks`]) or the key's icon by name ([`KeyFace::symbol`], drawn
//! from the built-in Lucide icons, an unknown name as the fallback icon).
//! Text is anti-aliased (see [`crate::text`]), wrapped by width and cut with
//! `…`; characters the font does not have are shown as `?`.
//!
//! Live state is drawn as marks that do not depend on any module: an
//! indicator dot in the top-right corner, a progress bar along the bottom
//! edge and a toggle bar along the top edge. [`FrameCache`] keeps rendered
//! frames by face and size, so a face that comes back (e.g. a state that
//! flips between two values) is not drawn again.
//!
//! A state's further marks ([`crate::marks`]) are an accent background
//! from a fixed palette, a badge in the top left corner and a series drawn
//! as a line behind the texts. What the user chose for the key wins: its
//! background over the accent, its icon over the glyph and the symbol.
//!
//! A face's [`KeyAppearance`] sets its background and text colours, the
//! title's size and position (or hides it) and an icon, scaled to the key
//! (see [`crate::icons`]).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; coordinates are clamped before use"
)]

use std::collections::{HashMap, VecDeque};

use deckpilot_core::{FaceStyle, FontSize, Glyph, Indicator, KeyAppearance, KeyFace, TitleAlign};
use deckpilot_device::Frame;

use crate::icons::{self, Icons};
use crate::marks;
use crate::text::{self, Style, Weight};

const WHITE: [u8; 3] = [240, 240, 240];
/// The value line: a little softer than the title.
const MUTED: [u8; 3] = [200, 200, 208];
const ACCENT: [u8; 3] = [56, 189, 248];
const GREEN: [u8; 3] = [34, 197, 94];
const AMBER: [u8; 3] = [245, 158, 11];
const RED: [u8; 3] = [239, 68, 68];
const BLUE: [u8; 3] = [59, 130, 246];
const GREY: [u8; 3] = [113, 113, 122];
const TRACK: [u8; 3] = [24, 24, 27];

fn indicator_color(indicator: Indicator) -> [u8; 3] {
    match indicator {
        Indicator::Ok => GREEN,
        Indicator::Warning => AMBER,
        Indicator::Error => RED,
        Indicator::Running => BLUE,
        Indicator::Unknown | Indicator::Loading => GREY,
    }
}

/// Backgrounds close to the black of a Stream Deck's screens: the icon
/// carries the key, the background only tells kinds of keys apart.
fn background(style: FaceStyle) -> [u8; 3] {
    match style {
        FaceStyle::Empty => [0, 0, 0],
        FaceStyle::Action => [28, 28, 32],
        FaceStyle::Workflow => [22, 32, 50],
        FaceStyle::Folder => [36, 34, 84],
        FaceStyle::Back => [40, 40, 46],
        FaceStyle::Notice => [66, 32, 10],
    }
}

/// Renders a face at `width` x `height` pixels, without icons.
pub fn render(face: &KeyFace, width: u32, height: u32) -> Frame {
    render_with(face, width, height, &Icons::default())
}

/// The title's em size on a key whose shorter side is `side` pixels.
fn title_size(size: FontSize, key_side: f32) -> f32 {
    let share = match size {
        FontSize::Small => 0.13,
        FontSize::Medium => 0.155,
        FontSize::Large => 0.19,
    };
    (key_side * share).max(6.0)
}

/// What the key's main mark is, when it is not the user's own icon.
#[derive(Debug, Clone, Copy)]
enum Mark<'a> {
    Glyph(Glyph),
    Symbol(&'a str),
}

/// A square, `(left, top, side)`.
type Square = (f32, f32, f32);

/// Where the mark and the lines of text go on a key.
#[derive(Debug, Default)]
struct Layout {
    mark: Option<Square>,
    /// Lines with their style, colour and top.
    lines: Vec<(String, Style, [u8; 3], f32)>,
    /// Top and bottom of the text block, if there is text.
    block: Option<(f32, f32)>,
}

struct Request<'a> {
    face: &'a KeyFace,
    look: &'a KeyAppearance,
    has_mark: bool,
    over_icon: bool,
}

#[expect(
    clippy::too_many_lines,
    reason = "one key's layout, read top to bottom"
)]
fn layout(request: &Request<'_>, width: u32, height: u32) -> Layout {
    let Request {
        face,
        look,
        has_mark,
        over_icon,
    } = *request;
    let (w, h) = (width as f32, height as f32);
    let side = w.min(h);
    let pad = (side * 0.07).max(2.0);
    // Room for the toggle mark at the top and the progress bar at the
    // bottom.
    let margin = (h / 12.0).max(3.0);
    let inner = (h - 2.0 * margin).max(0.0);
    let text_width = (w - 2.0 * pad).max(1.0);
    let ink = look.text_color.map_or(WHITE, deckpilot_core::Color::rgb);
    let value_ink = if face.highlighted {
        ACCENT
    } else {
        look.text_color.map_or(MUTED, deckpilot_core::Color::rgb)
    };
    let title_style = Style::new(
        Weight::Medium,
        title_size(look.font_size.unwrap_or_default(), side),
    );
    let value_style = Style::new(
        Weight::SemiBold,
        title_size(FontSize::Medium, side).max(title_style.size * 0.9),
    );
    let show_text = look.shows_title();
    let values = if show_text {
        value_lines(face.detail.as_deref(), value_style, text_width)
    } else {
        Vec::new()
    };
    let max_title =
        if has_mark { 2_usize } else { 3 }.saturating_sub(values.len().saturating_sub(1));
    let mut titles: Vec<String> = if show_text {
        title_style.wrap(&face.title, text_width, max_title.max(1))
    } else {
        Vec::new()
    };
    let block_height = |titles: usize| {
        titles as f32 * title_style.line_height()
            + values
                .iter()
                .map(|(_, style)| style.line_height())
                .sum::<f32>()
    };
    // Lines that fit on the key at all.
    while !titles.is_empty() && block_height(titles.len()) > inner {
        titles.pop();
    }
    let align = look.title_align.unwrap_or(if has_mark || over_icon {
        TitleAlign::Bottom
    } else {
        TitleAlign::Middle
    });
    let gap = (side * 0.05).max(1.0);
    let smallest_mark = side * 0.3;
    let largest_mark = if titles.is_empty() && values.is_empty() {
        side * 0.62
    } else {
        side * 0.46
    };
    // A two-line title gives way to the mark first.
    if has_mark && align != TitleAlign::Middle {
        while titles.len() > 1 && inner - block_height(titles.len()) - gap < smallest_mark {
            titles = title_style.wrap(&face.title, text_width, titles.len() - 1);
        }
    }
    let text_height = block_height(titles.len());
    let has_text = !titles.is_empty() || !values.is_empty();
    let block_top = match align {
        TitleAlign::Top => margin,
        TitleAlign::Middle => (h - text_height) / 2.0,
        TitleAlign::Bottom => h - margin - text_height,
    };
    let mark = has_mark.then(|| {
        let (from, to) = if has_text {
            match align {
                TitleAlign::Top => (block_top + text_height + gap, h - margin),
                // Centred text: the mark is not drawn over it.
                TitleAlign::Middle => (0.0, 0.0),
                TitleAlign::Bottom => (margin, block_top - gap),
            }
        } else {
            (margin, h - margin)
        };
        let room = (to - from).min(w - 2.0 * pad);
        (room >= side * 0.2).then(|| {
            let mark_side = room.min(largest_mark);
            (
                (w - mark_side) / 2.0,
                from + (to - from - mark_side) / 2.0,
                mark_side,
            )
        })
    });
    let mut lines = Vec::new();
    let mut y = block_top;
    for title in titles {
        lines.push((title, title_style, ink, y));
        y += title_style.line_height();
    }
    for (value, style) in values {
        lines.push((value, style, value_ink, y));
        y += style.line_height();
    }
    Layout {
        mark: mark.flatten(),
        lines,
        block: has_text.then_some((block_top, block_top + text_height)),
    }
}

/// The value lines of `detail` (at most two), each a little smaller when
/// it is too wide, then cut.
fn value_lines(detail: Option<&str>, style: Style, width: f32) -> Vec<(String, Style)> {
    detail
        .map(|detail| {
            detail
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(2)
                .map(|line| {
                    let style = style.shrunk_to(line, width, 0.8);
                    (style.fit(line, width), style)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Renders a face at `width` x `height` pixels, with its icon from `icons`.
pub fn render_with(face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Frame {
    let default_look = KeyAppearance::default();
    let look = face.appearance.as_ref().unwrap_or(&default_look);
    let fill = match (look.background, face.accent) {
        (Some(color), _) if !face.is_empty() => color.rgb(),
        (None, Some(accent)) if !face.is_empty() => marks::accent_color(accent),
        _ => background(face.style),
    };
    let mut frame = Frame::filled(width, height, fill);
    if face.is_empty() || width == 0 || height == 0 {
        return frame;
    }
    let icon = look
        .icon
        .and_then(|icon| Some((icons.get(icon.id)?, icon.fit)));
    if let Some((image, fit)) = icon {
        draw_icon(&mut frame, &icons::scale(image, width, height, fit));
    }
    let ink = look.text_color.map_or(WHITE, deckpilot_core::Color::rgb);
    if let Some(series) = &face.series {
        marks::draw_series(&mut frame, series, marks::lighten(fill, 35));
    }
    // The user's icon wins over a state's glyph, the glyph over the key's
    // own icon.
    let mark = if icon.is_some() {
        None
    } else if let Some(glyph) = face.glyph {
        Some(Mark::Glyph(glyph))
    } else {
        face.symbol.as_deref().map(Mark::Symbol)
    };
    let placed = layout(
        &Request {
            face,
            look,
            has_mark: mark.is_some(),
            over_icon: icon.is_some(),
        },
        width,
        height,
    );
    if let (Some(mark), Some((left, top, side))) = (mark, placed.mark) {
        match mark {
            Mark::Glyph(glyph) => marks::draw_glyph_in(&mut frame, glyph, (left, top, side), ink),
            Mark::Symbol(name) => text::draw_icon(&mut frame, name, left, top, side, ink),
        }
    }
    if icon.is_some()
        && let Some((top, bottom)) = placed.block
    {
        // Text over a picture sits on a translucent band, not a shadow.
        scrim(&mut frame, top - 1.0, bottom + 1.0);
    }
    let center = width as f32 / 2.0;
    for (line, style, color, top) in &placed.lines {
        style.draw_centered(&mut frame, line, center, *top, *color);
    }
    if face.highlighted {
        let thickness = (width.min(height) / 36).max(2);
        border(&mut frame, ACCENT, thickness);
    }
    draw_state(&mut frame, face);
    frame
}

/// Darkens the rows from `top` to `bottom` so that text on them reads on
/// any picture.
fn scrim(frame: &mut Frame, top: f32, bottom: f32) {
    let (from, to) = (
        top.max(0.0) as u32,
        (bottom.max(0.0) as u32).min(frame.height()),
    );
    for y in from..to {
        for x in 0..frame.width() {
            if let Some(under) = frame.pixel(x, y) {
                frame.put(x, y, text::mix(under, [0, 0, 0], 0.5));
            }
        }
    }
}

/// Blends an icon (as large as the frame) onto it.
fn draw_icon(frame: &mut Frame, icon: &image::RgbaImage) {
    for (x, y, pixel) in icon.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        if a == 0 {
            continue;
        }
        let Some(under) = frame.pixel(x, y) else {
            continue;
        };
        frame.put(x, y, text::mix(under, [r, g, b], f32::from(a) / 255.0));
    }
}

/// The indicator, progress, toggle and badge marks of a face.
fn draw_state(frame: &mut Frame, face: &KeyFace) {
    let (w, h) = (frame.width(), frame.height());
    if let Some(badge) = &face.badge {
        // Below the toggle mark, when there is one.
        let below = face.toggle.map_or(0, |_| (h / 24).max(2));
        marks::draw_badge(frame, badge, below, WHITE);
    }
    if let Some(on) = face.toggle {
        let thickness = (h / 24).max(2);
        let color = if on { GREEN } else { GREY };
        let length = if on { w } else { w / 3 };
        fill(frame, (w - length) / 2, 0, length, thickness, color);
    }
    if let Some(percent) = face.progress {
        let thickness = (h / 18).max(2);
        let y = h.saturating_sub(thickness);
        fill(frame, 0, y, w, thickness, TRACK);
        let done = w * u32::from(percent.min(100)) / 100;
        fill(frame, 0, y, done, thickness, BLUE);
    }
    if let Some(indicator) = face.indicator {
        let radius = (w.min(h) / 12).max(2);
        let margin = radius + (w.min(h) / 16).max(1);
        let center = (w.saturating_sub(margin + 1), margin);
        let color = indicator_color(indicator);
        // Loading is a ring, every other state a filled dot.
        let ring = indicator == Indicator::Loading;
        circle(frame, center, radius, color, ring);
    }
}

fn fill(frame: &mut Frame, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
    for dy in 0..height {
        for dx in 0..width {
            frame.put(x + dx, y + dy, color);
        }
    }
}

/// An anti-aliased dot (or ring) around `(cx, cy)`.
fn circle(frame: &mut Frame, (cx, cy): (u32, u32), radius: u32, color: [u8; 3], ring: bool) {
    let r = radius as f32;
    let inner = (r - (r / 3.0).max(1.2)).max(0.0);
    let reach = i64::from(radius) + 1;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            let outer = (r + 0.5 - distance).clamp(0.0, 1.0);
            let coverage = if ring {
                outer.min((distance - inner + 0.5).clamp(0.0, 1.0))
            } else {
                outer
            };
            text::blend_at(
                frame,
                (i64::from(cx) + dx) as f32,
                (i64::from(cy) + dy) as f32,
                color,
                coverage,
            );
        }
    }
}

/// Rendered frames by face and size, at most `capacity` of them (the
/// oldest are dropped first).
#[derive(Debug)]
pub struct FrameCache {
    capacity: usize,
    frames: HashMap<(KeyFace, u32, u32), Frame>,
    order: VecDeque<(KeyFace, u32, u32)>,
    rendered: usize,
}

impl FrameCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: HashMap::new(),
            order: VecDeque::new(),
            rendered: 0,
        }
    }

    /// The frame for `face` at `width` x `height`, rendered only if it is
    /// not cached. Icons are looked up in `icons`; call
    /// [`FrameCache::clear`] when they change.
    pub fn frame(&mut self, face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Frame {
        let key = (face.clone(), width, height);
        if let Some(frame) = self.frames.get(&key) {
            return frame.clone();
        }
        let frame = render_with(face, width, height, icons);
        self.rendered += 1;
        if self.frames.len() >= self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.frames.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.frames.insert(key, frame.clone());
        frame
    }

    /// Forgets every frame, e.g. after the profile's icons changed.
    pub fn clear(&mut self) {
        self.frames.clear();
        self.order.clear();
    }

    /// How many frames were drawn (cache misses).
    pub fn rendered(&self) -> usize {
        self.rendered
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

fn border(frame: &mut Frame, color: [u8; 3], thickness: u32) {
    let (w, h) = (frame.width(), frame.height());
    for y in 0..h {
        for x in 0..w {
            if x < thickness || y < thickness || x + thickness >= w || y + thickness >= h {
                frame.put(x, y, color);
            }
        }
    }
}

#[cfg(test)]
mod tests;
