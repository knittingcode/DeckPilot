//! Anti-aliased text and icons on keys, from outline fonts built into the
//! renderer (`assets/`, see `NOTICE`):
//!
//! - **Text**: Inter, medium for titles and semibold for values and badges,
//!   subset to Basic Latin, Latin-1, Latin Extended-A (Polish letters among
//!   them), common punctuation (`…`, dashes, typographic quotes), the
//!   arrows `← ↑ → ↓` and `€`. A character the font does not have is drawn
//!   as `?`. Kerning is applied; no hinting, shadow or extra spacing.
//! - **Icons**: the Lucide icons listed in `assets/icons.txt`, the set the
//!   app shows for actions, so a key on the device shows the icon the app
//!   shows. An unknown name is drawn as the fallback icon
//!   ([`FALLBACK_ICON`]), as in the app.
//!
//! Both are read from constant data compiled into the binary: no font file
//! is looked up or read at run time.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; coordinates are clamped before use"
)]

use std::sync::LazyLock;

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};
use deckpilot_device::Frame;

const MEDIUM: &[u8] = include_bytes!("../assets/Inter-Medium.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/Inter-SemiBold.ttf");
const ICONS: &[u8] = include_bytes!("../assets/Lucide-Keys.ttf");
const ICON_NAMES: &str = include_str!("../assets/icons.txt");

/// The icon drawn for a name the renderer does not know, as the app does.
pub const FALLBACK_ICON: &str = "zap";

/// A text weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    /// Titles.
    Medium,
    /// Values, badges.
    SemiBold,
}

struct Fonts {
    medium: FontRef<'static>,
    semibold: FontRef<'static>,
    icons: FontRef<'static>,
    /// Icon names in `icons.txt` order: icon `i` is `U+E000 + i`.
    names: Vec<&'static str>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("icons", &self.names.len())
            .finish_non_exhaustive()
    }
}

static FONTS: LazyLock<Fonts> = LazyLock::new(|| Fonts {
    // The fonts are part of the source tree; a broken one fails every
    // rendering test.
    medium: FontRef::try_from_slice(MEDIUM).expect("built-in title font"),
    semibold: FontRef::try_from_slice(SEMIBOLD).expect("built-in value font"),
    icons: FontRef::try_from_slice(ICONS).expect("built-in icon font"),
    names: ICON_NAMES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect(),
});

fn font(weight: Weight) -> &'static FontRef<'static> {
    match weight {
        Weight::Medium => &FONTS.medium,
        Weight::SemiBold => &FONTS.semibold,
    }
}

/// The scale at which the font's em is `px` pixels, as a CSS font size.
fn scale(font: &FontRef<'static>, px: f32) -> PxScale {
    let em = font.units_per_em().unwrap_or(1000.0);
    PxScale::from(px * font.height_unscaled() / em)
}

/// The glyph of `ch`, `?` for a character the font does not have.
fn glyph_of(font: &FontRef<'static>, ch: char) -> GlyphId {
    let id = font.glyph_id(ch);
    if id.0 == 0 { font.glyph_id('?') } else { id }
}

/// Text set at one size and weight.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Style {
    pub weight: Weight,
    /// The em size in pixels.
    pub size: f32,
}

impl Style {
    pub(crate) fn new(weight: Weight, size: f32) -> Self {
        Self {
            weight,
            size: size.max(1.0),
        }
    }

    /// How wide `text` is, with kerning.
    pub(crate) fn width(self, text: &str) -> f32 {
        let font = font(self.weight);
        let scaled = font.as_scaled(scale(font, self.size));
        let mut width = 0.0;
        let mut previous: Option<GlyphId> = None;
        for ch in text.chars().filter(|c| !c.is_control()) {
            let id = glyph_of(font, ch);
            if let Some(previous) = previous {
                width += scaled.kern(previous, id);
            }
            width += scaled.h_advance(id);
            previous = Some(id);
        }
        width
    }

    /// Height of a line: ascent to descent, without extra leading.
    pub(crate) fn line_height(self) -> f32 {
        let font = font(self.weight);
        let scaled = font.as_scaled(scale(font, self.size));
        scaled.ascent() - scaled.descent()
    }

    fn ascent(self) -> f32 {
        let font = font(self.weight);
        font.as_scaled(scale(font, self.size)).ascent()
    }

    /// Draws `text` with its line box's top at `top`, centred on `center_x`.
    pub(crate) fn draw_centered(
        self,
        frame: &mut Frame,
        text: &str,
        center_x: f32,
        top: f32,
        color: [u8; 3],
    ) {
        let left = center_x - self.width(text) / 2.0;
        self.draw(frame, text, left, top, color);
    }

    /// Draws `text` with its line box's top left corner at `(left, top)`.
    pub(crate) fn draw(self, frame: &mut Frame, text: &str, left: f32, top: f32, color: [u8; 3]) {
        let font = font(self.weight);
        let px = scale(font, self.size);
        let scaled = font.as_scaled(px);
        let baseline = top + self.ascent();
        let mut x = left;
        let mut previous: Option<GlyphId> = None;
        for ch in text.chars().filter(|c| !c.is_control()) {
            let id = glyph_of(font, ch);
            if let Some(previous) = previous {
                x += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(px, point(x, baseline));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    blend_at(
                        frame,
                        bounds.min.x + gx as f32,
                        bounds.min.y + gy as f32,
                        color,
                        coverage,
                    );
                });
            }
            x += scaled.h_advance(id);
            previous = Some(id);
        }
    }

    /// This style, made smaller (down to `least` of its size) so that
    /// `text` is at most `width` wide, if it can be.
    pub(crate) fn shrunk_to(self, text: &str, width: f32, least: f32) -> Self {
        let wide = self.width(text);
        if wide <= width || wide <= 0.0 {
            return self;
        }
        Self::new(self.weight, self.size * (width / wide).max(least))
    }

    /// `text` shortened with `…` until it is at most `width` wide (an
    /// empty string when not even `…` fits).
    pub(crate) fn fit(self, text: &str, width: f32) -> String {
        fit_with(text, |candidate| self.width(candidate) <= width)
    }

    /// `text` word-wrapped into at most `max_lines` lines of at most
    /// `width`; see [`wrap_with`].
    pub(crate) fn wrap(self, text: &str, width: f32, max_lines: usize) -> Vec<String> {
        wrap_with(text, max_lines, |candidate| self.width(candidate) <= width)
    }
}

/// `text` shortened with `…` until `fits` accepts it.
pub(crate) fn fit_with(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for keep in (0..chars.len()).rev() {
        let mut cut: String = chars[..keep].iter().collect();
        let trimmed = cut.trim_end().len();
        cut.truncate(trimmed);
        cut.push('…');
        if fits(&cut) {
            return cut;
        }
    }
    String::new()
}

/// Word-wraps `text` into at most `max_lines` lines that `fits` accepts.
/// A word longer than a line is broken; what does not fit on the last line
/// is cut there with `…`.
pub(crate) fn wrap_with(text: &str, max_lines: usize, fits: impl Fn(&str) -> bool) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut next = 0;
    // The rest of a word broken at the end of a line.
    let mut pending: Option<String> = None;
    while lines.len() < max_lines && (next < words.len() || pending.is_some()) {
        let last_line = lines.len() + 1 == max_lines;
        let mut current = String::new();
        loop {
            let word = match &pending {
                Some(rest) => rest.clone(),
                None => match words.get(next) {
                    Some(word) => (*word).to_owned(),
                    None => break,
                },
            };
            let candidate = if current.is_empty() {
                word.clone()
            } else {
                format!("{current} {word}")
            };
            if fits(&candidate) {
                current = candidate;
                if pending.take().is_none() {
                    next += 1;
                }
                continue;
            }
            if current.is_empty() && !last_line {
                // A word longer than a line: break it.
                let chars: Vec<char> = word.chars().collect();
                let mut take = chars.len().saturating_sub(1).max(1);
                while take > 1 && !fits(&chars[..take].iter().collect::<String>()) {
                    take -= 1;
                }
                current = chars[..take].iter().collect();
                let rest: String = chars[take..].iter().collect();
                if pending.is_none() {
                    next += 1;
                }
                pending = (!rest.is_empty()).then_some(rest);
            }
            break;
        }
        if last_line && (pending.is_some() || next < words.len()) {
            let rest: Vec<String> = [Some(current), pending.take()]
                .into_iter()
                .flatten()
                .chain(words[next..].iter().map(|word| (*word).to_owned()))
                .filter(|part| !part.is_empty())
                .collect();
            current = fit_with(&rest.join(" "), &fits);
            next = words.len();
        }
        lines.push(current);
    }
    lines
}

/// Whether `name` is an icon the renderer draws itself (not the fallback).
pub fn has_icon(name: &str) -> bool {
    FONTS.names.contains(&name)
}

/// Every icon name the renderer knows, in `assets/icons.txt` order.
pub fn icon_names() -> impl Iterator<Item = &'static str> {
    FONTS.names.iter().copied()
}

/// Draws the icon `name` (else [`FALLBACK_ICON`]) in the square at
/// `(left, top)` of `side` pixels.
pub(crate) fn draw_icon(
    frame: &mut Frame,
    name: &str,
    left: f32,
    top: f32,
    side: f32,
    color: [u8; 3],
) {
    let names = &FONTS.names;
    let index = names
        .iter()
        .position(|known| *known == name)
        .or_else(|| names.iter().position(|known| *known == FALLBACK_ICON));
    let Some(ch) = index
        .and_then(|index| u32::try_from(index).ok())
        .and_then(|index| char::from_u32(0xE000 + index))
    else {
        return;
    };
    let font = &FONTS.icons;
    let px = scale(font, side);
    let ascent = font.as_scaled(px).ascent();
    let glyph = font
        .glyph_id(ch)
        .with_scale_and_position(px, point(left, top + ascent));
    if let Some(outlined) = font.outline_glyph(glyph) {
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            blend_at(
                frame,
                bounds.min.x + gx as f32,
                bounds.min.y + gy as f32,
                color,
                coverage,
            );
        });
    }
}

/// Blends `color` over the pixel at `(x, y)` with `coverage` (0–1).
pub(crate) fn blend_at(frame: &mut Frame, x: f32, y: f32, color: [u8; 3], coverage: f32) {
    if x < 0.0 || y < 0.0 || coverage <= 0.0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    let Some(under) = frame.pixel(x, y) else {
        return;
    };
    frame.put(x, y, mix(under, color, coverage));
}

/// `under` moved towards `over` by `amount` (0–1); 1 is `over` exactly.
pub(crate) fn mix(under: [u8; 3], over: [u8; 3], amount: f32) -> [u8; 3] {
    let amount = amount.clamp(0.0, 1.0);
    if amount >= 0.996 {
        return over;
    }
    let mut out = [0; 3];
    for (channel, (u, o)) in out.iter_mut().zip(under.iter().zip(over.iter())) {
        let value = f32::from(*u) + (f32::from(*o) - f32::from(*u)) * amount;
        *channel = value.round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_chars(columns: usize) -> impl Fn(&str) -> bool {
        move |text: &str| text.chars().count() <= columns
    }

    #[test]
    fn wraps_and_truncates() {
        assert_eq!(
            wrap_with("Start Work", 4, by_chars(9)),
            vec!["Start", "Work"]
        );
        assert_eq!(
            wrap_with("Supercalifragilistic", 4, by_chars(9)),
            vec!["Supercali", "fragilist", "ic"]
        );
        let two = wrap_with("a b c d e f g h i j", 2, by_chars(1));
        assert_eq!(two.len(), 2);
        assert_eq!(two[1], "…");
        let cut = wrap_with("Open the project dashboard now", 2, by_chars(12));
        assert_eq!(cut, vec!["Open the", "project das…"]);
        assert_eq!(fit_with("ABCDEFGHIJK", by_chars(9)), "ABCDEFGH…");
        assert_eq!(fit_with("Short", by_chars(9)), "Short");
        assert_eq!(fit_with("Too long", by_chars(0)), "");
        assert_eq!(wrap_with("", 4, by_chars(9)), Vec::<String>::new());
    }

    #[test]
    fn text_is_measured_with_the_font() {
        let style = Style::new(Weight::Medium, 12.0);
        // Proportional: narrow letters are narrower than wide ones.
        assert!(style.width("iiii") < style.width("WWWW"));
        // Twice the size, about twice the width.
        let double = Style::new(Weight::Medium, 24.0).width("Disk");
        assert!((double / style.width("Disk") - 2.0).abs() < 0.05);
        // Kerning: "AV" is tighter than its letters apart.
        assert!(style.width("AV") < style.width("A") + style.width("V"));
        // Missing characters take the width of `?`.
        assert!((style.width("中") - style.width("?")).abs() < f32::EPSILON);
        assert!(style.line_height() > 12.0 && style.line_height() < 18.0);
        // Polish letters are in the font, not replaced.
        for ch in "ĄĆĘŁŃÓŚŹŻąćęłńóśźż↓↑…€".chars() {
            assert_ne!(FONTS.medium.glyph_id(ch).0, 0, "{ch}");
            assert_ne!(FONTS.semibold.glyph_id(ch).0, 0, "{ch}");
        }
    }

    #[test]
    fn text_is_anti_aliased() {
        let mut frame = Frame::filled(60, 20, [0, 0, 0]);
        Style::new(Weight::Medium, 12.0).draw(&mut frame, "Mgła", 2.0, 2.0, [255, 255, 255]);
        let shades: std::collections::BTreeSet<u8> = (0..20)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .filter_map(|(x, y)| frame.pixel(x, y))
            .map(|p| p[0])
            .collect();
        // Black, white and many shades in between.
        assert!(shades.len() > 20, "{shades:?}");
        // Nothing outside the frame panics.
        Style::new(Weight::SemiBold, 40.0).draw(&mut frame, "Overflow", -30.0, -10.0, [9, 9, 9]);
    }

    #[test]
    fn icons_are_drawn_by_name_with_a_fallback() {
        let draw = |name: &str| {
            let mut frame = Frame::filled(32, 32, [0, 0, 0]);
            draw_icon(&mut frame, name, 2.0, 2.0, 28.0, [255, 255, 255]);
            frame
        };
        let lit = |frame: &Frame| {
            (0..32)
                .flat_map(|y| (0..32).map(move |x| (x, y)))
                .filter(|(x, y)| frame.pixel(*x, *y) != Some([0, 0, 0]))
                .count()
        };
        let mut seen = std::collections::BTreeSet::new();
        for name in icon_names() {
            let frame = draw(name);
            assert!(lit(&frame) > 30, "{name}");
            // Inside its square.
            for y in 0..32 {
                for x in 0..32 {
                    let inside = (1..31).contains(&x) && (1..31).contains(&y);
                    assert!(
                        inside || frame.pixel(x, y) == Some([0, 0, 0]),
                        "{name} {x},{y}"
                    );
                }
            }
            assert!(seen.insert(frame.rgb().to_vec()), "{name} repeats an icon");
        }
        assert!(has_icon("cpu") && has_icon(FALLBACK_ICON));
        assert!(!has_icon("no-such-icon"));
        assert_eq!(draw("no-such-icon").rgb(), draw(FALLBACK_ICON).rgb());
    }

    /// Every icon the app offers for actions (`lib/icons.ts`) is drawn on
    /// the device as the app shows it, not as the fallback.
    #[test]
    fn the_apps_action_icons_are_all_built_in() {
        let source = include_str!("../../../apps/desktop/src/lib/icons.ts");
        let start = source
            .find("export const ACTION_ICONS")
            .expect("ACTION_ICONS in lib/icons.ts");
        let block = &source[start..];
        let block = &block[..block.find("\n}").expect("end of ACTION_ICONS")];
        let names: Vec<&str> = block
            .lines()
            .skip(1)
            .filter_map(|line| line.trim().split_once(':'))
            .map(|(name, _)| name.trim().trim_matches('"'))
            .collect();
        assert!(names.len() > 40, "{names:?}");
        for name in names {
            assert!(
                has_icon(name),
                "`{name}` is offered by the app but not built in"
            );
        }
    }

    #[test]
    fn mixing() {
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 1.0), [200, 100, 50]);
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 0.0), [0, 0, 0]);
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 0.5), [100, 50, 25]);
    }
}
