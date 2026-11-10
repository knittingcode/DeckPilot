//! Renders [`KeyFace`]s into device-independent [`Frame`]s.
//!
//! The renderer only needs the target size, so the same code serves any key
//! or display size. Text uses a built-in 8 x 8 bitmap font (ASCII); other
//! characters are shown as `?`.

use deckpilot_core::{FaceStyle, KeyFace};
use deckpilot_device::Frame;
use font8x8::legacy::BASIC_LEGACY;

const GLYPH: u32 = 8;
const WHITE: [u8; 3] = [240, 240, 240];
const MUTED: [u8; 3] = [160, 160, 170];
const ACCENT: [u8; 3] = [56, 189, 248];

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
    frame
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
