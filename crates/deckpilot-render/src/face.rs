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
//! Text is anti-aliased (see [`crate::text`]) and laid out by the binding's
//! text mode (wrapped by default, on one line, fitted or scrolling; the
//! rules are in `docs/key-text.md`), never past the text area; characters
//! the font does not have are shown as `?`. [`text_report`] says how a
//! title came out, [`marquee`] how it scrolls.
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
//!
//! Colours a key does not set come from its device model's
//! [`DefaultAppearance`], the same for every kind of key: [`key_colors`]
//! resolves them in one place (the key's own colour, else its state's
//! accent, else the model's default) and a transparent background as the
//! display background behind it, so the device and the app's previews
//! show the same pixels. [`render`] and [`render_with`] draw with
//! [`DefaultAppearance::BUILTIN`] (the Stream Deck's), [`render_for`] with
//! a model's own.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; coordinates are clamped before use"
)]

use std::collections::{HashMap, VecDeque};

use deckpilot_core::{
    Background, DefaultAppearance, FitLines, FontSize, Glyph, Indicator, KeyAppearance, KeyFace,
    TextAlign, TextAppearance, TextMode, TextOverflow, TitleAlign,
};
use deckpilot_device::Frame;

use crate::icons::{self, BackgroundLayer, Icons};
use crate::marks;
use crate::marquee::Marquee;
use crate::text::{self, Style, Weight};

/// A badge's text, on its own dark plate.
const WHITE: [u8; 3] = [255, 255, 255];
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

/// The colours a key is drawn with, all opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyColors {
    /// What the key is filled with (a transparent background resolved to
    /// the display background).
    pub background: [u8; 3],
    /// The title, the icon by name and a state's glyph.
    pub text: [u8; 3],
    /// The value line under the title.
    pub value: [u8; 3],
}

/// The colours of `face` on a model whose defaults are `defaults`, in one
/// place for the device and the app's previews:
///
/// - background: the key's own (colour or transparent), else its state's
///   accent, else the model's default; transparent shows the model's
///   display background. An empty key has only the default. The kind of
///   key (action, folder, Back…) does not change it. A key whose own
///   background is an asset (a gradient or an image, drawn over this
///   colour) has the model's default here: under the asset, and instead
///   of it while the asset is not available.
/// - text: the key's own, else the model's default.
/// - value line: the accent of a highlighted key, else the key's text
///   colour, else the default text colour a fifth of the way to the
///   display background (a little softer than the title).
pub fn key_colors(face: &KeyFace, defaults: &DefaultAppearance) -> KeyColors {
    let look = face.appearance.as_ref();
    let own = look
        .and_then(|look| look.background.as_ref())
        .filter(|_| !face.is_empty());
    let accent = face.accent.filter(|_| !face.is_empty());
    let background = match (own, accent) {
        (Some(Background::Plain(own)), _) => own.over(defaults.display_background).rgb(),
        // An asset is the key's own choice too: it beats the accent.
        (Some(Background::Asset(_)), _) | (None, None) => defaults.effective_background(None).rgb(),
        (None, Some(accent)) => marks::accent_color(accent),
    };
    let own_text = look
        .and_then(|look| look.text_color)
        .map(deckpilot_core::Color::rgb);
    let text = own_text.unwrap_or(defaults.text_color.rgb());
    let value = if face.highlighted {
        ACCENT
    } else {
        own_text.unwrap_or_else(|| {
            text::mix(
                defaults.text_color.rgb(),
                defaults.display_background.rgb(),
                0.2,
            )
        })
    };
    KeyColors {
        background,
        text,
        value,
    }
}

/// Renders a face at `width` x `height` pixels, without icons, with the
/// built-in defaults.
pub fn render(face: &KeyFace, width: u32, height: u32) -> Frame {
    render_with(face, width, height, &Icons::default())
}

/// Largest icon above a title, as a share of the key's shorter side.
pub(crate) const MARK_WITH_TEXT: f32 = 0.42;
/// Largest icon on a key without text.
pub(crate) const MARK_ALONE: f32 = 0.54;

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

/// A line of text placed on the key.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub text: String,
    pub style: Style,
    pub color: [u8; 3],
    pub left: f32,
    pub top: f32,
    /// Columns the line may draw in (from, to): set for clipped and
    /// scrolling lines, which are wider than the text area.
    pub clip: Option<(f32, f32)>,
}

/// Where the mark and the lines of text go on a key.
#[derive(Debug, Default)]
struct Layout {
    mark: Option<Square>,
    lines: Vec<Line>,
    /// Top and bottom of the text block, if there is text.
    block: Option<(f32, f32)>,
    /// How the title was laid out; `None` when the face shows no title.
    report: Option<TextReport>,
}

struct Request<'a> {
    face: &'a KeyFace,
    look: &'a KeyAppearance,
    colors: KeyColors,
    has_mark: bool,
    over_icon: bool,
}

/// How a key's title came out (`docs/key-text.md`), for the app's
/// Inspector: whether all of it is shown, at which size, on how many
/// lines, and whether it scrolls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextReport {
    pub mode: TextMode,
    /// All of the title is shown: nothing cut with `…` or clipped, no line
    /// left out (a scrolling title fits: it shows everything in turn). For
    /// auto fit also: no word broken.
    pub fits: bool,
    /// The title is drawn cut: with `…`, clipped, or lines left out.
    pub truncated: bool,
    /// The size the title is drawn at, in points (pixels on a 72 px key).
    pub font_points: f32,
    /// The size the title is drawn at, in pixels of the frame.
    pub font_pixels: f32,
    pub lines: usize,
    /// A marquee title wider than the key: it scrolls on the device.
    pub needs_scroll: bool,
    /// How far a scrolling title moves, in pixels of the frame.
    pub scroll_distance: f32,
}

/// Pixels on a key whose shorter side is `side` for `points` (pixels on a
/// 72 px key).
pub(crate) fn points_to_pixels(points: f32, side: f32) -> f32 {
    (points * side / 72.0).max(1.0)
}

/// What a title is laid out in.
struct TitleRoom {
    /// The width of the text area.
    width: f32,
    /// Height for the title and the value lines.
    inner: f32,
    values_height: f32,
    /// The most lines when the mode does not set them: two above an icon,
    /// three without (fewer with a two-line value).
    auto_lines: usize,
    /// The title gives way to the mark (an icon above or below it).
    mark_below: bool,
    gap: f32,
    smallest_mark: f32,
    side: f32,
}

/// A laid-out title.
struct Title {
    lines: Vec<String>,
    style: Style,
    /// Lines wider than the text area, drawn clipped from its start.
    clipped: bool,
    /// The scroll offset to draw a scrolling title at.
    offset: Option<f32>,
    report: TextReport,
}

/// Steps auto fit tries between the preferred and the smallest size.
const FIT_STEPS: f32 = 48.0;

/// The title of `face` in `room`, by the rules of its text mode
/// (`docs/key-text.md`). `scroll`: the offset a scrolling title is drawn
/// at; `None` draws it still (cut with `…`).
#[expect(
    clippy::too_many_lines,
    reason = "one branch per text mode, read top to bottom"
)]
fn lay_out_title(
    title: &str,
    text: &TextAppearance,
    preferred: Style,
    room: &TitleRoom,
    scroll: Option<f32>,
) -> Title {
    let width = room.width;
    let block =
        |style: Style, lines: usize| lines as f32 * style.line_height() + room.values_height;
    // Room left for the title when it must leave the smallest mark room.
    let mark_room = if room.mark_below {
        room.gap + room.smallest_mark
    } else {
        0.0
    };
    let report = |style: Style, lines: usize, truncated: bool| TextReport {
        mode: text.mode,
        fits: !truncated,
        truncated,
        font_points: style.size * 72.0 / room.side.max(1.0),
        font_pixels: style.size,
        lines,
        needs_scroll: false,
        scroll_distance: 0.0,
    };
    let one_line = || title.split_whitespace().collect::<Vec<_>>().join(" ");
    let wrap_into = |style: Style, max_lines: usize| -> (text::Wrapped, bool) {
        let mut wrapped = style.wrapped(title, width, max_lines);
        let mut dropped = false;
        // Lines that do not fit on the key at all.
        while !wrapped.lines.is_empty() && block(style, wrapped.lines.len()) > room.inner {
            wrapped.lines.pop();
            dropped = true;
        }
        (wrapped, dropped)
    };
    let title_of = |lines: Vec<String>, style: Style, report: TextReport| Title {
        lines,
        style,
        clipped: false,
        offset: None,
        report,
    };
    match text.mode {
        TextMode::Wrap => {
            let Some(max_lines) = text.max_lines() else {
                // As keys always wrapped: two lines above an icon, three
                // without, a second line given up for the mark first.
                let (mut wrapped, mut dropped) = wrap_into(preferred, room.auto_lines);
                if room.mark_below {
                    while wrapped.lines.len() > 1
                        && room.inner - block(preferred, wrapped.lines.len()) - room.gap
                            < room.smallest_mark
                    {
                        (wrapped, dropped) = wrap_into(preferred, wrapped.lines.len() - 1);
                    }
                }
                let cut = wrapped.truncated || dropped;
                let lines = wrapped.lines.len();
                return title_of(wrapped.lines, preferred, report(preferred, lines, cut));
            };
            // As many lines as asked for that fit on the key; the last one
            // that fits ends with `…` when text is left.
            let fit = ((room.inner - room.values_height) / preferred.line_height()).floor();
            let max_lines = max_lines.min(fit.max(0.0) as usize);
            let wrapped = preferred.wrapped(title, width, max_lines);
            let lines = wrapped.lines.len();
            title_of(
                wrapped.lines,
                preferred,
                report(preferred, lines, wrapped.truncated),
            )
        }
        TextMode::SingleLine => {
            let line = one_line();
            if line.is_empty() || block(preferred, 1) > room.inner {
                let cut = !line.is_empty();
                return title_of(Vec::new(), preferred, report(preferred, 0, cut));
            }
            let wide = preferred.width(&line) > width;
            match text.overflow() {
                TextOverflow::Ellipsis => {
                    let shown = preferred.fit(&line, width);
                    title_of(vec![shown], preferred, report(preferred, 1, wide))
                }
                TextOverflow::Clip => Title {
                    lines: vec![line],
                    style: preferred,
                    clipped: wide,
                    offset: None,
                    report: report(preferred, 1, wide),
                },
            }
        }
        TextMode::AutoFit => {
            let least =
                points_to_pixels(f32::from(text.min_points()), room.side).min(preferred.size);
            let step = ((preferred.size - least) / FIT_STEPS).max(0.25);
            let sizes = std::iter::successors(Some(preferred.size), |size| {
                (*size - step > least).then_some(*size - step)
            })
            .chain(std::iter::once(least));
            let available = room.inner - mark_room;
            match text.fit() {
                FitLines::SingleLine => {
                    let line = one_line();
                    if line.is_empty() {
                        return title_of(Vec::new(), preferred, report(preferred, 0, false));
                    }
                    // The size it fits at exactly, a hair less so rounding
                    // never cuts it; then the steps below that.
                    let exact = preferred.size * (width / preferred.width(&line).max(0.01)) * 0.995;
                    let found = sizes
                        .filter(|size| *size <= exact.max(least))
                        .map(|size| preferred.with_size(size))
                        .find(|style| style.width(&line) <= width && block(*style, 1) <= available);
                    if let Some(style) = found {
                        return title_of(vec![line], style, report(style, 1, false));
                    }
                    // Not even at the smallest size: as large as allowed
                    // there, cut with `…`, and reported.
                    let style = preferred.with_size(least);
                    if block(style, 1) > room.inner {
                        return title_of(Vec::new(), style, report(style, 0, true));
                    }
                    let shown = style.fit(&line, width);
                    title_of(vec![shown], style, report(style, 1, true))
                }
                FitLines::Multiline => {
                    let max_lines = text.max_lines().unwrap_or(room.auto_lines);
                    let found = sizes
                        .map(|size| preferred.with_size(size))
                        .find_map(|style| {
                            let wrapped = style.wrapped(title, width, max_lines);
                            (!wrapped.truncated
                                && !wrapped.broken
                                && block(style, wrapped.lines.len()) <= available)
                                .then_some((style, wrapped))
                        });
                    if let Some((style, wrapped)) = found {
                        let lines = wrapped.lines.len();
                        return title_of(wrapped.lines, style, report(style, lines, false));
                    }
                    let style = preferred.with_size(least);
                    let (wrapped, _) = wrap_into(style, max_lines);
                    let lines = wrapped.lines.len();
                    let mut report = report(style, lines, true);
                    // All text shown, only a word broken: shown whole, but
                    // still not a fit.
                    report.truncated = wrapped.truncated || lines == 0;
                    report.fits = false;
                    title_of(wrapped.lines, style, report)
                }
            }
        }
        TextMode::Marquee => {
            let line = one_line();
            if line.is_empty() || block(preferred, 1) > room.inner {
                let cut = !line.is_empty();
                return title_of(Vec::new(), preferred, report(preferred, 0, cut));
            }
            let distance = preferred.width(&line) - width;
            if distance <= 0.0 {
                return title_of(vec![line], preferred, report(preferred, 1, false));
            }
            let mut scrolling = report(preferred, 1, false);
            scrolling.needs_scroll = true;
            scrolling.scroll_distance = distance;
            match scroll {
                // Still (the app's preview, reduced motion): cut with `…`.
                None => title_of(vec![preferred.fit(&line, width)], preferred, scrolling),
                Some(offset) => Title {
                    lines: vec![line],
                    style: preferred,
                    clipped: true,
                    offset: Some(offset.clamp(0.0, distance.ceil())),
                    report: scrolling,
                },
            }
        }
    }
}

fn layout(request: &Request<'_>, width: u32, height: u32) -> Layout {
    layout_scrolled(request, width, height, None)
}

/// The layout of a key; `scroll` places a scrolling title (see
/// [`lay_out_title`]).
#[expect(
    clippy::too_many_lines,
    reason = "one key's layout, read top to bottom"
)]
fn layout_scrolled(request: &Request<'_>, width: u32, height: u32, scroll: Option<f32>) -> Layout {
    let Request {
        face,
        look,
        colors,
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
    let (ink, value_ink) = (colors.text, colors.value);
    let text = look.text();
    let preferred = text.size_points().map_or_else(
        || title_size(look.font_size.unwrap_or_default(), side),
        |points| points_to_pixels(f32::from(points), side),
    );
    // Bold is the heavier built-in face; italic and underline are drawn
    // (`text.rs`). The font family is Inter: the only title font
    // (`deckpilot_core::TITLE_FONTS`).
    let title_style = Style::new(
        if look.is_bold() {
            Weight::SemiBold
        } else {
            Weight::Medium
        },
        preferred,
    )
    .styled(look.is_italic(), look.is_underlined());
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
    let align = look.title_align.unwrap_or(if has_mark || over_icon {
        TitleAlign::Bottom
    } else {
        TitleAlign::Middle
    });
    let gap = (side * 0.06).max(1.0);
    let smallest_mark = side * 0.3;
    let values_height: f32 = values.iter().map(|(_, style)| style.line_height()).sum();
    let title = show_text.then(|| {
        lay_out_title(
            &face.title,
            &text,
            title_style,
            &TitleRoom {
                width: text_width,
                inner,
                values_height,
                auto_lines: max_title.max(1),
                mark_below: has_mark && align != TitleAlign::Middle,
                gap,
                smallest_mark,
                side,
            },
            scroll,
        )
    });
    let (titles, title_style) = title.as_ref().map_or((&[][..], title_style), |title| {
        (&title.lines[..], title.style)
    });
    // The icon leads but does not crowd the key: about two fifths of it
    // above a title, a little over half on its own.
    let largest_mark = if titles.is_empty() && values.is_empty() {
        side * MARK_ALONE
    } else {
        side * MARK_WITH_TEXT
    };
    let text_height = titles.len() as f32 * title_style.line_height() + values_height;
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
    let horizontal = text.align();
    let place = |line: &str, style: Style| -> f32 {
        let wide = style.width(line);
        match horizontal {
            TextAlign::Left => pad,
            TextAlign::Center => (w - wide) / 2.0,
            TextAlign::Right => w - pad - wide,
        }
    };
    let area = (pad, w - pad);
    let mut lines = Vec::new();
    let mut y = block_top;
    for line in titles {
        let (left, clip) = match &title {
            Some(Title {
                offset: Some(offset),
                ..
            }) => (pad - offset, Some(area)),
            // Too wide: clipped from the start of the text area.
            Some(Title { clipped: true, .. }) => (pad, Some(area)),
            _ => (place(line, title_style), None),
        };
        lines.push(Line {
            text: line.clone(),
            style: title_style,
            color: ink,
            left,
            top: y,
            clip,
        });
        y += title_style.line_height();
    }
    for (value, style) in values {
        lines.push(Line {
            left: place(&value, style),
            text: value,
            style,
            color: value_ink,
            top: y,
            clip: None,
        });
        y += style.line_height();
    }
    Layout {
        mark: mark.flatten(),
        lines,
        block: has_text.then_some((block_top, block_top + text_height)),
        report: title.map(|title| title.report),
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

/// Renders a face at `width` x `height` pixels, with its icon from `icons`
/// and the built-in defaults ([`DefaultAppearance::BUILTIN`]). A scrolling
/// title is drawn still, cut with `…` (see [`render_scrolled`]).
pub fn render_with(face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Frame {
    render_for(face, width, height, icons, &DefaultAppearance::BUILTIN)
}

/// Renders a face as [`render_with`] does, for a model whose defaults are
/// `defaults` ([`deckpilot_device::DeviceModel::effective_appearance`]).
pub fn render_for(
    face: &KeyFace,
    width: u32,
    height: u32,
    icons: &Icons,
    defaults: &DefaultAppearance,
) -> Frame {
    draw_face(face, width, height, icons, defaults, None)
}

/// Renders a face whose title scrolls ([`marquee`]) with the title moved
/// `offset` pixels of the frame to the left ([`Marquee::offset_at`]), for
/// a model whose defaults are `defaults`; a face whose title does not
/// scroll is drawn as by [`render_for`].
pub fn render_scrolled(
    face: &KeyFace,
    width: u32,
    height: u32,
    icons: &Icons,
    defaults: &DefaultAppearance,
    offset: f32,
) -> Frame {
    draw_face(face, width, height, icons, defaults, Some(offset))
}

/// The icon the user chose for a key, as it is drawn now.
#[derive(Debug, Clone, Copy)]
enum OwnIcon<'a> {
    /// An image (My Icons, a plugin's), covering the key by its fit.
    Image(&'a image::RgbaImage, deckpilot_core::IconFit),
    /// A built-in library icon, drawn from the font as the key's mark.
    Symbol(&'a str),
}

/// The key's own icon when it can be drawn: a reference to an asset that
/// is not there (a plugin's that is disabled, an icon of a later version)
/// draws none, and the key looks as if it set no icon.
fn own_icon<'a>(look: &'a KeyAppearance, icons: &'a Icons) -> Option<OwnIcon<'a>> {
    let icon = look.icon.as_ref()?;
    match &icon.id {
        deckpilot_core::AssetRef::Library(id) if id.is_builtin() => {
            text::has_icon(id.name()).then(|| OwnIcon::Symbol(id.name()))
        }
        reference => icons
            .image(reference)
            .map(|image| OwnIcon::Image(image, icon.fit)),
    }
}

/// The mark a face is drawn with: what the layout depends on besides the
/// face and whether it is drawn over a picture.
fn mark_of<'a>(face: &'a KeyFace, own: Option<OwnIcon<'a>>) -> Option<Mark<'a>> {
    // The user's icon wins over a state's glyph, the glyph over the key's
    // own icon.
    match own {
        Some(OwnIcon::Image(..)) => None,
        Some(OwnIcon::Symbol(name)) => Some(Mark::Symbol(name)),
        None => face
            .glyph
            .map(Mark::Glyph)
            .or_else(|| face.symbol.as_deref().map(Mark::Symbol)),
    }
}

/// The key's background asset as it is drawn now (`None` when it is not
/// there: the default background shows instead).
fn background_layer<'a>(look: &'a KeyAppearance, icons: &'a Icons) -> Option<BackgroundLayer<'a>> {
    icons.background(look.background.as_ref()?.asset()?)
}

/// How the title of `face` comes out at `width` x `height` (`None` when
/// the face shows no title: empty, or the title hidden). Lays the key out
/// without drawing it; the app shows it in the Inspector.
pub fn text_report(face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Option<TextReport> {
    if face.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let default_look = KeyAppearance::default();
    let look = face.appearance.as_ref().unwrap_or(&default_look);
    // As drawn: an icon that is not in `icons` is not drawn.
    let own = own_icon(look, icons);
    let over_icon = matches!(own, Some(OwnIcon::Image(..)));
    layout(
        &Request {
            face,
            look,
            // Colours do not change how the title is laid out.
            colors: key_colors(face, &DefaultAppearance::BUILTIN),
            has_mark: mark_of(face, own).is_some(),
            over_icon,
        },
        width,
        height,
    )
    .report
}

/// The scrolling of the title of `face` at `width` x `height`, when it
/// is a marquee title wider than the key; `None` when it stands still.
pub fn marquee(face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Option<Marquee> {
    let report = text_report(face, width, height, icons)?;
    if !report.needs_scroll {
        return None;
    }
    let text = face
        .appearance
        .as_ref()
        .map(KeyAppearance::text)
        .unwrap_or_default();
    Some(Marquee::new(
        report.scroll_distance,
        f32::from(text.speed()),
        std::time::Duration::from_millis(u64::from(text.pause_ms())),
    ))
}

/// Whether `face` is the folder's protected Back key: its own vector icon
/// (`assets/icons/folder-back.svg`), centred, without text, on the
/// model's default background like every other key.
fn is_folder_back(face: &KeyFace) -> bool {
    face.symbol.as_deref() == Some(deckpilot_core::device_runtime::symbols::FOLDER_BACK)
        && face.glyph.is_none()
}

fn draw_face(
    face: &KeyFace,
    width: u32,
    height: u32,
    icons: &Icons,
    defaults: &DefaultAppearance,
    scroll: Option<f32>,
) -> Frame {
    let default_look = KeyAppearance::default();
    let look = face.appearance.as_ref().unwrap_or(&default_look);
    let own = own_icon(look, icons);
    let folder_back = is_folder_back(face) && own.is_none();
    let colors = key_colors(face, defaults);
    let fill = colors.background;
    let mut frame = Frame::filled(width, height, fill);
    if face.is_empty() || width == 0 || height == 0 {
        return frame;
    }
    let layer = background_layer(look, icons);
    if let Some(layer) = layer {
        draw_background(&mut frame, layer);
    }
    let image = match own {
        Some(OwnIcon::Image(image, fit)) => Some((image, fit)),
        _ => None,
    };
    if let Some((image, fit)) = image {
        draw_icon(&mut frame, &icons::scale(image, width, height, fit));
    }
    // Text over a picture (the icon or a background image) sits on a band.
    let over_picture = image.is_some() || matches!(layer, Some(BackgroundLayer::Image(_)));
    let ink = colors.text;
    if let Some(series) = &face.series {
        marks::draw_series(&mut frame, series, marks::lighten(fill, 35));
    }
    if folder_back && let Some(back) = crate::vector::folder_back() {
        // Drawn from the vector at this size: sharp on every key.
        let side = width.min(height) as f32 * MARK_ALONE;
        let (left, top) = ((width as f32 - side) / 2.0, (height as f32 - side) / 2.0);
        back.draw(&mut frame, left, top, side, ink);
    }
    let mark = mark_of(face, own).filter(|_| !folder_back);
    let placed = layout_scrolled(
        &Request {
            face,
            look,
            colors,
            has_mark: mark.is_some(),
            over_icon: image.is_some(),
        },
        width,
        height,
        scroll,
    );
    if let (Some(mark), Some((left, top, side))) = (mark, placed.mark) {
        match mark {
            Mark::Glyph(glyph) => marks::draw_glyph_in(&mut frame, glyph, (left, top, side), ink),
            Mark::Symbol(name) => text::draw_icon(&mut frame, name, left, top, side, ink),
        }
    }
    if over_picture && let Some((top, bottom)) = placed.block {
        // Text over a picture sits on a translucent band, not a shadow.
        scrim(&mut frame, top - 1.0, bottom + 1.0);
    }
    for line in &placed.lines {
        line.style.draw_clipped(
            &mut frame,
            &line.text,
            (line.left, line.top),
            line.color,
            line.clip,
        );
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

/// Paints a background asset over the whole frame: a gradient, or an
/// image covering the key (cropped, aspect ratio kept) and blended over
/// the fill where it is translucent.
fn draw_background(frame: &mut Frame, layer: BackgroundLayer<'_>) {
    match layer {
        BackgroundLayer::Gradient(gradient) => crate::gradient::paint(frame, gradient),
        BackgroundLayer::Image(image) => draw_icon(
            frame,
            &icons::scale(
                image,
                frame.width(),
                frame.height(),
                deckpilot_core::IconFit::Crop,
            ),
        ),
    }
}

/// The background `background` alone at `width` x `height`, on a model
/// whose defaults are `defaults`: what a key with it and nothing else
/// shows, for the Background Library's tiles. `None` (and an asset that is
/// not here) is the model's default.
pub fn background_preview(
    background: Option<&Background>,
    width: u32,
    height: u32,
    icons: &Icons,
    defaults: &DefaultAppearance,
) -> Frame {
    let plain = background.and_then(Background::plain);
    let mut frame = Frame::filled(width, height, defaults.effective_background(plain).rgb());
    if let Some(layer) = background
        .and_then(Background::asset)
        .and_then(|reference| icons.background(reference))
    {
        draw_background(&mut frame, layer);
    }
    frame
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
        let radius = (w.min(h) / 16).max(2);
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

/// What a cached frame was drawn from: the face, the size and the model's
/// defaults.
type CacheKey = (KeyFace, u32, u32, DefaultAppearance);

/// Rendered frames by face, size and model defaults, at most `capacity`
/// of them (the oldest are dropped first).
#[derive(Debug)]
pub struct FrameCache {
    capacity: usize,
    frames: HashMap<CacheKey, Frame>,
    order: VecDeque<CacheKey>,
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

    /// The frame for `face` at `width` x `height` on a model whose
    /// defaults are `defaults`, rendered only if it is not cached. Icons
    /// are looked up in `icons`; call [`FrameCache::clear`] when they
    /// change.
    pub fn frame(
        &mut self,
        face: &KeyFace,
        width: u32,
        height: u32,
        icons: &Icons,
        defaults: &DefaultAppearance,
    ) -> Frame {
        let key = (face.clone(), width, height, *defaults);
        if let Some(frame) = self.frames.get(&key) {
            return frame.clone();
        }
        let frame = render_for(face, width, height, icons, defaults);
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

#[cfg(test)]
mod text_modes;

#[cfg(test)]
mod title_style;
