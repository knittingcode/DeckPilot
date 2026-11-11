//! Renders [`KeyFace`]s into device-independent [`Frame`]s.
//!
//! The renderer only needs the target size, so the same code serves any key
//! or display size. Text uses a built-in 8 x 8 bitmap font (ASCII); other
//! characters are shown as `?`.
//!
//! Live state is drawn as marks that do not depend on any module: an
//! indicator dot in the top-right corner, a progress bar along the bottom
//! edge and a toggle bar along the top edge. [`FrameCache`] keeps rendered
//! frames by face and size, so a face that comes back (e.g. a state that
//! flips between two values) is not drawn again.

use std::collections::{HashMap, VecDeque};

use deckpilot_core::{FaceStyle, Indicator, KeyFace};
use deckpilot_device::Frame;
use font8x8::legacy::BASIC_LEGACY;

const GLYPH: u32 = 8;
const WHITE: [u8; 3] = [240, 240, 240];
const MUTED: [u8; 3] = [160, 160, 170];
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

fn background(style: FaceStyle) -> [u8; 3] {
    match style {
        FaceStyle::Empty => [0, 0, 0],
        FaceStyle::Action => [39, 39, 42],
        FaceStyle::Workflow => [30, 41, 59],
        FaceStyle::Folder => [49, 46, 129],
        FaceStyle::Back => [63, 63, 70],
        FaceStyle::Notice => [69, 26, 3],
    }
}

/// Renders a face at `width` x `height` pixels.
pub fn render(face: &KeyFace, width: u32, height: u32) -> Frame {
    let mut frame = Frame::filled(width, height, background(face.style));
    if face.is_empty() {
        return frame;
    }
    if face.highlighted {
        border(&mut frame, ACCENT, 3);
    }

    let scale = if width >= 120 { 2 } else { 1 };
    let columns = ((width.saturating_sub(4)) / (GLYPH * scale)).max(1) as usize;
    let mut lines = wrap(&face.title, columns, 4);
    let detail = face.detail.as_deref().map(|d| truncate(d, columns));
    let line_height = GLYPH * scale + 2;
    let total_lines = lines.len() + usize::from(detail.is_some());
    let block = u32::try_from(total_lines).unwrap_or(0) * line_height;
    let mut y = height.saturating_sub(block) / 2;

    for line in lines.drain(..) {
        draw_centered(&mut frame, &line, y, scale, WHITE);
        y += line_height;
    }
    if let Some(detail) = detail {
        draw_centered(
            &mut frame,
            &detail,
            y,
            scale,
            if face.highlighted { ACCENT } else { MUTED },
        );
    }
    draw_state(&mut frame, face);
    frame
}

/// The indicator, progress and toggle marks of a face.
fn draw_state(frame: &mut Frame, face: &KeyFace) {
    let (w, h) = (frame.width(), frame.height());
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

fn circle(frame: &mut Frame, (cx, cy): (u32, u32), radius: u32, color: [u8; 3], ring: bool) {
    let r = i64::from(radius);
    let inner = (r - 1).max(0);
    for dy in -r..=r {
        for dx in -r..=r {
            let distance = dx * dx + dy * dy;
            if distance > r * r || (ring && distance < inner * inner) {
                continue;
            }
            let (Ok(x), Ok(y)) = (
                u32::try_from(i64::from(cx) + dx),
                u32::try_from(i64::from(cy) + dy),
            ) else {
                continue;
            };
            frame.put(x, y, color);
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
    /// not cached.
    pub fn frame(&mut self, face: &KeyFace, width: u32, height: u32) -> Frame {
        let key = (face.clone(), width, height);
        if let Some(frame) = self.frames.get(&key) {
            return frame.clone();
        }
        let frame = render(face, width, height);
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

fn truncate(text: &str, columns: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= columns {
        text.to_owned()
    } else {
        let mut cut: String = chars[..columns.saturating_sub(1)].iter().collect();
        cut.push('.');
        cut
    }
}

/// Word-wraps `text` into at most `max_lines` lines of `columns` characters.
fn wrap(text: &str, columns: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_owned();
        loop {
            let needed = if current.is_empty() {
                word.chars().count()
            } else {
                current.chars().count() + 1 + word.chars().count()
            };
            if needed <= columns {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&word);
                break;
            }
            if current.is_empty() {
                // A single word longer than a line: hard-break it.
                let head: String = word.chars().take(columns).collect();
                word = word.chars().skip(columns).collect();
                lines.push(head);
                if word.is_empty() {
                    break;
                }
            } else {
                lines.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = truncate(&format!("{last}  "), columns);
        }
    }
    lines
}

fn draw_centered(frame: &mut Frame, text: &str, y: u32, scale: u32, color: [u8; 3]) {
    let width = u32::try_from(text.chars().count()).unwrap_or(0) * GLYPH * scale;
    let mut x = frame.width().saturating_sub(width) / 2;
    for ch in text.chars() {
        let glyph = BASIC_LEGACY
            .get(ch as usize)
            .or(BASIC_LEGACY.get(usize::from(b'?')));
        if let Some(rows) = glyph {
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..GLYPH {
                    if bits & (1 << column) != 0 {
                        let row = u32::try_from(row).unwrap_or(0);
                        for dy in 0..scale {
                            for dx in 0..scale {
                                frame.put(x + column * scale + dx, y + row * scale + dy, color);
                            }
                        }
                    }
                }
            }
        }
        x += GLYPH * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(frame: &Frame, background: [u8; 3]) -> usize {
        (0..frame.height())
            .flat_map(|y| (0..frame.width()).map(move |x| (x, y)))
            .filter(|(x, y)| frame.pixel(*x, *y) != Some(background))
            .count()
    }

    #[test]
    fn empty_faces_are_black() {
        let frame = render(&KeyFace::empty(), 72, 72);
        assert_eq!(lit(&frame, [0, 0, 0]), 0);
    }

    #[test]
    fn text_is_drawn_within_bounds_for_any_size() {
        let face = KeyFace::new(FaceStyle::Action, "Start Work").with_detail("SHOP");
        for (w, h) in [(72, 72), (96, 96), (120, 120), (200, 100), (8, 8)] {
            let frame = render(&face, w, h);
            assert_eq!((frame.width(), frame.height()), (w, h));
            if w >= 72 {
                assert!(lit(&frame, background(FaceStyle::Action)) > 20);
            }
        }
    }

    #[test]
    fn highlight_draws_a_border() {
        let face = KeyFace::new(FaceStyle::Action, "CMS").highlighted(true);
        let frame = render(&face, 72, 72);
        assert_eq!(frame.pixel(0, 0), Some(ACCENT));
        assert_eq!(frame.pixel(71, 71), Some(ACCENT));
    }

    #[test]
    fn state_marks_are_drawn() {
        let plain = KeyFace::new(FaceStyle::Action, "Repo");
        let base = render(&plain, 72, 72);
        let marked = |face: &KeyFace| {
            let frame = render(face, 72, 72);
            (0..72u32)
                .flat_map(|y| (0..72u32).map(move |x| (x, y)))
                .filter(|(x, y)| frame.pixel(*x, *y) != base.pixel(*x, *y))
                .count()
        };
        for indicator in [
            Indicator::Ok,
            Indicator::Warning,
            Indicator::Error,
            Indicator::Running,
            Indicator::Unknown,
            Indicator::Loading,
        ] {
            let face = plain.clone().with_indicator(indicator);
            assert!(marked(&face) > 4, "{indicator:?}");
            let frame = render(&face, 72, 72);
            let color = indicator_color(indicator);
            assert!(
                (0..72u32).any(|x| (0..20u32).any(|y| frame.pixel(x, y) == Some(color))),
                "{indicator:?}"
            );
        }
        // Progress fills the bottom edge in proportion.
        let half = render(&plain.clone().with_progress(Some(50)), 72, 72);
        assert_eq!(half.pixel(10, 71), Some(BLUE));
        assert_eq!(half.pixel(60, 71), Some(TRACK));
        let mut toggled = plain.clone();
        toggled.toggle = Some(true);
        assert_eq!(render(&toggled, 72, 72).pixel(36, 0), Some(GREEN));
        // Tiny keys do not panic.
        let mut all = plain
            .with_indicator(Indicator::Error)
            .with_progress(Some(100));
        all.toggle = Some(false);
        for (w, h) in [(1, 1), (8, 8), (200, 100)] {
            let frame = render(&all, w, h);
            assert_eq!((frame.width(), frame.height()), (w, h));
        }
    }

    #[test]
    fn frames_are_cached_by_face_and_size() {
        let mut cache = FrameCache::new(2);
        let a = KeyFace::new(FaceStyle::Action, "A");
        let b = KeyFace::new(FaceStyle::Action, "B");
        let c = KeyFace::new(FaceStyle::Action, "C");
        let first = cache.frame(&a, 72, 72);
        assert_eq!(cache.frame(&a, 72, 72), first);
        assert_eq!(cache.rendered(), 1);
        cache.frame(&a, 96, 96);
        assert_eq!(cache.rendered(), 2);
        // A different state is a different face.
        cache.frame(&a.clone().with_indicator(Indicator::Ok), 72, 72);
        assert_eq!(cache.rendered(), 3);
        cache.frame(&b, 72, 72);
        cache.frame(&c, 72, 72);
        assert_eq!(cache.len(), 2);
        // `a` was dropped as the oldest.
        cache.frame(&a, 72, 72);
        assert_eq!(cache.rendered(), 6);
    }

    #[test]
    fn wraps_and_truncates() {
        assert_eq!(wrap("Start Work", 9, 4), vec!["Start", "Work"]);
        assert_eq!(
            wrap("Supercalifragilistic", 9, 4),
            vec!["Supercali", "fragilist", "ic"]
        );
        assert_eq!(wrap("a b c d e f g h i j", 1, 2).len(), 2);
        assert_eq!(truncate("ABCDEFGHIJK", 9), "ABCDEFGH.");
        assert_eq!(wrap("", 9, 4), Vec::<String>::new());
    }
}
