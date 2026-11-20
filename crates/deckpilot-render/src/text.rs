//! Anti-aliased text and icons on keys, from outline fonts built into the
//! renderer (`assets/`, see `NOTICE`):
//!
//! - **Text**: Inter, medium for titles and semibold for values and badges,
//!   subset to Basic Latin, Latin-1, Latin Extended-A (Polish letters among
//!   them), common punctuation (`…`, dashes, typographic quotes), the
//!   arrows `← ↑ → ↓` and `€`. A character the font does not have is drawn
//!   as `?`. Kerning is applied; no hinting, shadow or extra spacing.
//!   Outlines are read with `skrifa` and filled by `ab_glyph_rasterizer`.
//! - **Icons**: the Lucide icons listed in `assets/icons.txt`, the set the
//!   app shows for actions, so a key on the device shows the icon the app
//!   shows. An unknown name is drawn as the fallback icon
//!   ([`FALLBACK_ICON`]), as in the app.
//!
//! Both are read from constant data compiled into the binary: no font file
//! is looked up or read at run time.
//!
//! Key titles may be bold, italic and underlined (`docs/key-text.md`, "Title
//! appearance"), drawn from these faces alone so every device and the app's
//! previews show the same: bold is Inter semibold (no heavier face is built
//! in), italic is Inter slanted by 10° (no italic face is built in; Inter's
//! own italic leans 9.4°), measured with its overhang so it never leaves the
//! text area, and the underline follows the font's underline position and
//! thickness.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; coordinates are clamped before use"
)]

use std::sync::LazyLock;

use ab_glyph_rasterizer::{Point, Rasterizer, point};
use deckpilot_device::Frame;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::kern::SubtableKind;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use unicode_segmentation::UnicodeSegmentation as _;

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
    medium: FontRef::new(MEDIUM).expect("built-in title font"),
    semibold: FontRef::new(SEMIBOLD).expect("built-in value font"),
    icons: FontRef::new(ICONS).expect("built-in icon font"),
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

/// The glyph of `ch`, `?` for a character the font does not have.
fn glyph_of(font: &FontRef<'static>, ch: char) -> GlyphId {
    let charmap = font.charmap();
    charmap
        .map(ch)
        .filter(|id| *id != GlyphId::NOTDEF)
        .or_else(|| charmap.map('?'))
        .unwrap_or(GlyphId::NOTDEF)
}

/// The kerning between two glyphs in font units (the `kern` table that
/// `assets/make-fonts.py` writes from the font's pair adjustments).
fn kerning(font: &FontRef<'static>, left: GlyphId, right: GlyphId) -> f32 {
    let Ok(kern) = font.kern() else {
        return 0.0;
    };
    kern.subtables()
        .flatten()
        .filter(|subtable| subtable.is_horizontal() && !subtable.is_cross_stream())
        .find_map(|subtable| match subtable.kind() {
            Ok(SubtableKind::Format0(pairs)) => pairs.kerning(left, right),
            _ => None,
        })
        .map_or(0.0, |value| value as f32)
}

/// How far an italic title leans: x moves right by this much per pixel
/// above the baseline (tan 10°).
pub(crate) const ITALIC_SLANT: f32 = 0.176_327;

/// The title fonts of [`deckpilot_core::TITLE_FONTS`] the renderer has
/// faces for, by id: every listed font must be here (tested), and a font
/// not listed is drawn with the first.
const FAMILIES: &[&str] = &["inter"];

/// Whether the renderer has faces for the title font `id`.
pub fn has_title_font(id: &str) -> bool {
    FAMILIES.contains(&id)
}

/// One glyph's outline as segments in frame pixels (y down).
#[derive(Debug, Default)]
struct Segments {
    origin: (f32, f32),
    /// Leans the outline right above the baseline (italic).
    slant: f32,
    start: Point,
    current: Point,
    lines: Vec<[Point; 2]>,
    quads: Vec<[Point; 3]>,
    cubics: Vec<[Point; 4]>,
}

impl Segments {
    fn at(&self, x: f32, y: f32) -> Point {
        point(self.origin.0 + x + y * self.slant, self.origin.1 - y)
    }

    fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.lines
            .iter()
            .flatten()
            .chain(self.quads.iter().flatten())
            .chain(self.cubics.iter().flatten())
            .copied()
    }

    /// Fills the outline into `frame` in `color`, anti-aliased, only in
    /// the columns of `clip` (from, to) when given.
    fn fill(&self, frame: &mut Frame, color: [u8; 3], clip: Option<(f32, f32)>) {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in self.points() {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        if min_x > max_x {
            return;
        }
        if let Some((from, to)) = clip
            && (max_x < from || min_x >= to)
        {
            return;
        }
        let (x0, y0) = (min_x.floor(), min_y.floor());
        let (width, height) = ((max_x - x0).ceil() + 1.0, (max_y - y0).ceil() + 1.0);
        if width > 4096.0 || height > 4096.0 {
            return;
        }
        let mut raster = Rasterizer::new(width as usize, height as usize);
        let local = |p: Point| point(p.x - x0, p.y - y0);
        for [a, b] in &self.lines {
            raster.draw_line(local(*a), local(*b));
        }
        for [a, b, c] in &self.quads {
            raster.draw_quad(local(*a), local(*b), local(*c));
        }
        for [a, b, c, d] in &self.cubics {
            raster.draw_cubic(local(*a), local(*b), local(*c), local(*d));
        }
        raster.for_each_pixel_2d(|x, y, coverage| {
            let x = x0 + x as f32;
            if clip.is_none_or(|(from, to)| x >= from.floor() && x < to.ceil()) {
                blend_at(frame, x, y0 + y as f32, color, coverage);
            }
        });
    }
}

impl OutlinePen for Segments {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.at(x, y);
        self.current = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.at(x, y);
        self.lines.push([self.current, to]);
        self.current = to;
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let to = self.at(x, y);
        self.quads.push([self.current, self.at(cx0, cy0), to]);
        self.current = to;
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let to = self.at(x, y);
        self.cubics
            .push([self.current, self.at(cx0, cy0), self.at(cx1, cy1), to]);
        self.current = to;
    }

    fn close(&mut self) {
        if self.current != self.start {
            self.lines.push([self.current, self.start]);
        }
        self.current = self.start;
    }
}

/// Draws `glyph` of `font` at `size` pixels per em with its origin at `x`
/// on the baseline `baseline`.
#[expect(
    clippy::too_many_arguments,
    reason = "one glyph's placement, all needed"
)]
fn draw_glyph(
    frame: &mut Frame,
    font: &FontRef<'static>,
    glyph: GlyphId,
    size: f32,
    (x, baseline): (f32, f32),
    slant: f32,
    color: [u8; 3],
    clip: Option<(f32, f32)>,
) {
    let Some(outline) = font.outline_glyphs().get(glyph) else {
        return;
    };
    let mut segments = Segments {
        origin: (x, baseline),
        slant,
        ..Segments::default()
    };
    let settings = DrawSettings::unhinted(Size::new(size), LocationRef::default());
    if outline.draw(settings, &mut segments).is_ok() {
        segments.fill(frame, color, clip);
    }
}

/// Text set at one size and weight, upright or italic, underlined or not.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Style {
    pub weight: Weight,
    /// The em size in pixels.
    pub size: f32,
    pub italic: bool,
    pub underline: bool,
}

impl Style {
    pub(crate) fn new(weight: Weight, size: f32) -> Self {
        Self {
            weight,
            size: size.max(1.0),
            italic: false,
            underline: false,
        }
    }

    /// This style at another size.
    pub(crate) fn with_size(self, size: f32) -> Self {
        Self {
            size: size.max(1.0),
            ..self
        }
    }

    /// This style, italic and underlined or not.
    pub(crate) fn styled(self, italic: bool, underline: bool) -> Self {
        Self {
            italic,
            underline,
            ..self
        }
    }

    fn slant(self) -> f32 {
        if self.italic { ITALIC_SLANT } else { 0.0 }
    }

    /// How far italic ink reaches past the advance (the top of the tallest
    /// letters leans right): counted in the width, so slanted text never
    /// leaves the area it was fitted to.
    fn overhang(self) -> f32 {
        self.ascent() * self.slant()
    }

    /// The glyphs of `text` with the pen position of each, and the width.
    fn layout(self, text: &str) -> (Vec<(GlyphId, f32)>, f32) {
        let font = font(self.weight);
        let metrics = font.glyph_metrics(Size::new(self.size), LocationRef::default());
        let em = font
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em
            .max(1);
        let units = self.size / f32::from(em);
        let mut x = 0.0;
        let mut previous: Option<GlyphId> = None;
        let mut glyphs = Vec::new();
        // One glyph per grapheme cluster: its first character (a letter
        // with combining marks shows the letter, an emoji sequence one
        // `?`), so no cluster is ever drawn in pieces.
        for ch in text
            .graphemes(true)
            .filter_map(|cluster| cluster.chars().next())
            .filter(|c| !c.is_control())
        {
            let id = glyph_of(font, ch);
            if let Some(previous) = previous {
                x += kerning(font, previous, id) * units;
            }
            glyphs.push((id, x));
            x += metrics.advance_width(id).unwrap_or(0.0);
            previous = Some(id);
        }
        (glyphs, x)
    }

    /// How wide `text` is, with kerning (and an italic's overhang).
    pub(crate) fn width(self, text: &str) -> f32 {
        let advance = self.layout(text).1;
        if advance > 0.0 {
            advance + self.overhang()
        } else {
            advance
        }
    }

    fn ascent(self) -> f32 {
        font(self.weight)
            .metrics(Size::new(self.size), LocationRef::default())
            .ascent
    }

    /// Height of a line: ascent to descent, without extra leading.
    pub(crate) fn line_height(self) -> f32 {
        let metrics = font(self.weight).metrics(Size::new(self.size), LocationRef::default());
        metrics.ascent - metrics.descent
    }

    /// Draws `text` with its line box's top left corner at `(left, top)`.
    pub(crate) fn draw(self, frame: &mut Frame, text: &str, left: f32, top: f32, color: [u8; 3]) {
        self.draw_clipped(frame, text, (left, top), color, None);
    }

    /// Draws `text` with its line box's top left corner at `(left, top)`,
    /// only in the columns of `clip` (from, to) when given: a clipped or
    /// scrolling line never draws outside the text area.
    pub(crate) fn draw_clipped(
        self,
        frame: &mut Frame,
        text: &str,
        (left, top): (f32, f32),
        color: [u8; 3],
        clip: Option<(f32, f32)>,
    ) {
        let font = font(self.weight);
        let baseline = top + self.ascent();
        let (glyphs, advance) = self.layout(text);
        for (glyph, x) in glyphs {
            draw_glyph(
                frame,
                font,
                glyph,
                self.size,
                (left + x, baseline),
                self.slant(),
                color,
                clip,
            );
        }
        if self.underline && advance > 0.0 {
            let (offset, thickness) = self.underline_metrics();
            // Inside the line box, whatever the font says.
            let bottom = top + self.line_height();
            let from = (baseline - offset).min(bottom - thickness);
            fill_rect(
                frame,
                (left, from),
                (left + advance, from + thickness),
                color,
                clip,
            );
        }
    }

    /// The underline's offset (from the baseline up to its top; negative
    /// below) and thickness in pixels, from the font; at least a pixel thick.
    fn underline_metrics(self) -> (f32, f32) {
        let metrics = font(self.weight).metrics(Size::new(self.size), LocationRef::default());
        let (offset, thickness) = metrics
            .underline
            .map_or((-0.09 * self.size, 0.07 * self.size), |line| {
                (line.offset, line.thickness)
            });
        (offset, thickness.max(1.0))
    }

    /// This style, made smaller (down to `least` of its size) so that
    /// `text` is at most `width` wide, if it can be.
    pub(crate) fn shrunk_to(self, text: &str, width: f32, least: f32) -> Self {
        let wide = self.width(text);
        if wide <= width || wide <= 0.0 {
            return self;
        }
        // A hair below the exact ratio, so rounding never cuts it after all.
        self.with_size(self.size * (width / wide * 0.995).max(least))
    }

    /// `text` shortened with `…` until it is at most `width` wide (an
    /// empty string when not even `…` fits).
    pub(crate) fn fit(self, text: &str, width: f32) -> String {
        fit_with(text, |candidate| self.width(candidate) <= width)
    }

    /// `text` word-wrapped into at most `max_lines` lines of at most
    /// `width` (see [`wrap_report`]), with whether text was cut or a word
    /// broken.
    pub(crate) fn wrapped(self, text: &str, width: f32, max_lines: usize) -> Wrapped {
        wrap_report(text, max_lines, |candidate| self.width(candidate) <= width)
    }
}

/// The grapheme clusters of `text`: what a reader sees as one character.
fn clusters(text: &str) -> Vec<&str> {
    text.graphemes(true).collect()
}

/// `text` shortened with `…` until `fits` accepts it. Cut between
/// grapheme clusters only, so an emoji or a letter with its accents is
/// never split; spaces before the `…` are dropped. Empty when not even `…`
/// fits.
pub(crate) fn fit_with(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_owned();
    }
    let parts = clusters(text);
    for keep in (0..parts.len()).rev() {
        let mut cut = parts[..keep].concat();
        let trimmed = cut.trim_end().len();
        cut.truncate(trimmed);
        cut.push('…');
        if fits(&cut) {
            return cut;
        }
    }
    String::new()
}

/// Lines of wrapped text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Wrapped {
    pub lines: Vec<String>,
    /// Text was left out: the last line ends with `…` (or is empty when
    /// not even `…` fits).
    pub truncated: bool,
    /// A word was broken because it alone is wider than a line.
    pub broken: bool,
}

/// A piece of text to wrap: a word, or an explicit line break.
#[derive(Debug)]
enum Token<'a> {
    Word(Vec<&'a str>),
    Break,
}

/// Word-wraps `text` into at most `max_lines` lines that `fits` accepts.
/// A word longer than a line is broken; what does not fit on the last line
/// is cut there with `…`.
#[cfg(test)]
pub(crate) fn wrap_with(text: &str, max_lines: usize, fits: impl Fn(&str) -> bool) -> Vec<String> {
    wrap_report(text, max_lines, fits).lines
}

/// Word-wraps `text` into at most `max_lines` lines that `fits` accepts
/// (`docs/key-text.md`):
///
/// - lines break between words (any whitespace), greedily;
/// - an explicit line break (`\n`) always ends a line; empty lines between
///   paragraphs are kept, leading and trailing ones dropped;
/// - a word wider than a line alone is broken between grapheme clusters,
///   as late as possible;
/// - when more lines would be needed, the last line holds the rest of the
///   text (line breaks read as spaces), cut with `…`.
pub(crate) fn wrap_report(text: &str, max_lines: usize, fits: impl Fn(&str) -> bool) -> Wrapped {
    let mut tokens: Vec<Token<'_>> = Vec::new();
    for (index, paragraph) in text.lines().enumerate() {
        if index > 0 {
            tokens.push(Token::Break);
        }
        tokens.extend(
            paragraph
                .split_whitespace()
                .map(|w| Token::Word(clusters(w))),
        );
    }
    // No empty lines before the first word or after the last one.
    while matches!(tokens.last(), Some(Token::Break)) {
        tokens.pop();
    }
    let lead = tokens
        .iter()
        .take_while(|token| matches!(token, Token::Break))
        .count();
    tokens.drain(..lead);

    let mut wrapped = Wrapped::default();
    if max_lines == 0 {
        wrapped.truncated = !tokens.is_empty();
        return wrapped;
    }
    // Every line with the position its text starts at: a token, and a
    // cluster in it (a word broken at the end of the line before).
    let mut lines: Vec<(String, (usize, usize))> = Vec::new();
    let mut at = (0, 0);
    while at.0 < tokens.len() {
        let start = at;
        let mut current = String::new();
        loop {
            match tokens.get(at.0) {
                None => break,
                Some(Token::Break) => {
                    at = (at.0 + 1, 0);
                    break;
                }
                Some(Token::Word(parts)) => {
                    let word = &parts[at.1..];
                    let candidate = if current.is_empty() {
                        word.concat()
                    } else {
                        format!("{current} {}", word.concat())
                    };
                    if fits(&candidate) {
                        current = candidate;
                        at = (at.0 + 1, 0);
                        continue;
                    }
                    if current.is_empty() {
                        // A word longer than a line: break it.
                        let mut take = word.len().saturating_sub(1).max(1);
                        while take > 1 && !fits(&word[..take].concat()) {
                            take -= 1;
                        }
                        current = word[..take].concat();
                        if take < word.len() {
                            at.1 += take;
                            wrapped.broken = true;
                        } else {
                            at = (at.0 + 1, 0);
                        }
                    }
                    break;
                }
            }
        }
        lines.push((current, start));
    }
    if lines.len() > max_lines {
        let (from, offset) = lines[max_lines - 1].1;
        let rest: Vec<String> = tokens[from..]
            .iter()
            .enumerate()
            .filter_map(|(index, token)| match token {
                Token::Word(parts) => {
                    let skip = if index == 0 { offset } else { 0 };
                    Some(parts[skip..].concat())
                }
                Token::Break => None,
            })
            .filter(|part| !part.is_empty())
            .collect();
        let rest = rest.join(" ");
        let cut = fit_with(&rest, &fits);
        wrapped.truncated = cut != rest;
        lines.truncate(max_lines - 1);
        lines.push((cut, (from, offset)));
    }
    wrapped.lines = lines.into_iter().map(|(line, _)| line).collect();
    wrapped
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
    let ascent = font.metrics(Size::new(side), LocationRef::default()).ascent;
    draw_glyph(
        frame,
        font,
        glyph_of(font, ch),
        side,
        (left, top + ascent),
        0.0,
        color,
        None,
    );
}

/// Fills the rectangle from `(x0, y0)` to `(x1, y1)` (fractions blended),
/// only in the columns of `clip` (from, to) when given.
fn fill_rect(
    frame: &mut Frame,
    (mut x0, y0): (f32, f32),
    (mut x1, y1): (f32, f32),
    color: [u8; 3],
    clip: Option<(f32, f32)>,
) {
    if let Some((from, to)) = clip {
        x0 = x0.max(from);
        x1 = x1.min(to);
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let overlap = |cell: f32, from: f32, to: f32| (to.min(cell + 1.0) - from.max(cell)).max(0.0);
    let mut y = y0.floor();
    while y < y1 {
        let rows = overlap(y, y0, y1);
        let mut x = x0.floor();
        while x < x1 {
            blend_at(frame, x, y, color, rows * overlap(x, x0, x1));
            x += 1.0;
        }
        y += 1.0;
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

    /// Wrapping before text modes (character based, line breaks read as
    /// spaces), kept to check that titles without line breaks wrap exactly
    /// as they did.
    fn wrap_before_text_modes(
        text: &str,
        max_lines: usize,
        fits: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        fn fit_before(text: &str, fits: impl Fn(&str) -> bool) -> String {
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
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut next = 0;
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
                current = fit_before(&rest.join(" "), &fits);
                next = words.len();
            }
            lines.push(current);
        }
        lines
    }

    /// The default (wrap) mode draws titles without line breaks exactly as
    /// before: the same lines for every width and line count, measured
    /// by characters and with the real font.
    #[test]
    fn wrapping_without_line_breaks_is_unchanged() {
        let titles = [
            "",
            "Mail",
            "Start Work",
            "Open the project dashboard now",
            "Supercalifragilisticexpialidocious",
            "a b c d e f g h i j",
            "Git SHOP status — main ahead 3",
            "Zażółć gęślą jaźń",
            "ĄĆĘŁŃÓŚŹŻ ąćęłńóśźż",
            "  spaced   out\twords  ",
            "x  Supercalifragilistic y",
            "Docker compose up --build",
        ];
        let style = Style::new(Weight::Medium, 11.16);
        for title in titles {
            for max_lines in 1..=4 {
                for columns in 1..=14 {
                    assert_eq!(
                        wrap_with(title, max_lines, by_chars(columns)),
                        wrap_before_text_modes(title, max_lines, by_chars(columns)),
                        "{title:?} {max_lines} {columns}"
                    );
                }
                for width in [20.0, 41.9, 61.92, 90.0, 140.0] {
                    let fits = |text: &str| style.width(text) <= width;
                    assert_eq!(
                        wrap_with(title, max_lines, fits),
                        wrap_before_text_modes(title, max_lines, fits),
                        "{title:?} {max_lines} {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn line_breaks_end_lines() {
        assert_eq!(
            wrap_with("Build\nDeploy", 4, by_chars(20)),
            ["Build", "Deploy"]
        );
        assert_eq!(wrap_with("a\r\nb", 4, by_chars(20)), ["a", "b"]);
        // Empty lines between paragraphs stay, at the ends they go.
        assert_eq!(wrap_with("\na\n\nb\n", 4, by_chars(20)), ["a", "", "b"]);
        // Too many lines: the rest on the last one, breaks as spaces.
        let cut = wrap_report("one\ntwo\nthree\nfour", 2, by_chars(9));
        assert_eq!(cut.lines, ["one", "two thre…"]);
        assert!(cut.truncated && !cut.broken);
        let fits = wrap_report("one\ntwo", 1, by_chars(9));
        assert_eq!(fits.lines, ["one two"]);
        assert!(!fits.truncated);
        // A broken word is reported, a wrapped one is not.
        assert!(wrap_report("Supercalifragilistic", 4, by_chars(9)).broken);
        assert!(!wrap_report("Start Work", 4, by_chars(9)).broken);
        assert!(wrap_report("text", 0, by_chars(9)).truncated);
        assert_eq!(wrap_report("", 0, by_chars(9)), Wrapped::default());
    }

    #[test]
    fn cuts_fall_between_grapheme_clusters() {
        // A family emoji is one cluster of seven characters; a flag two
        // regional indicators; é here a letter and a combining accent.
        let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let text = format!("ab{family}🇵🇱e\u{301}xyz");
        for columns in 0..12 {
            let cut = fit_with(&text, by_chars(columns));
            let kept = cut.trim_end_matches('…');
            assert!(text.starts_with(kept));
            assert!(
                text.grapheme_indices(true)
                    .map(|(at, _)| at)
                    .chain(std::iter::once(text.len()))
                    .any(|at| at == kept.len()),
                "{columns}: {cut:?}"
            );
        }
        // Broken words too.
        let wrapped = wrap_with(&format!("{family}{family}{family}"), 4, by_chars(8));
        assert_eq!(wrapped.concat(), format!("{family}{family}{family}"));
        assert!(
            wrapped.iter().all(|line| line.starts_with('👨')),
            "{wrapped:?}"
        );
        // A cluster is one glyph: as wide as `?` when the font lacks it.
        let style = Style::new(Weight::Medium, 12.0);
        assert!((style.width(family) - style.width("?")).abs() < f32::EPSILON);
        assert!((style.width("e\u{301}") - style.width("e")).abs() < f32::EPSILON);
        assert!((style.width("Ż") - style.width("Z")).abs() < 2.0);
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
            for font in [&FONTS.medium, &FONTS.semibold] {
                assert!(
                    font.charmap()
                        .map(ch)
                        .is_some_and(|id| id != GlyphId::NOTDEF),
                    "{ch}"
                );
            }
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
