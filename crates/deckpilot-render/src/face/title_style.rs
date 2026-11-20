//! A key title's look through the renderer (`docs/key-text.md`, "Title
//! appearance"): shown or hidden, top / middle / bottom, bold, italic,
//! underline, size, colour and font, in every text mode.

use deckpilot_core::{Color, FaceStyle, FontFamilyId, TITLE_FONTS, TextMode, TitleAlign};
use deckpilot_device::models;

use super::tests::ink_box;
use super::*;

const BG: [u8; 3] = [0, 0, 0];

fn face(title: &str, look: KeyAppearance) -> KeyFace {
    let mut face = KeyFace::new(FaceStyle::Action, title);
    face.appearance = look.normalized();
    face
}

fn ink(frame: &Frame) -> u32 {
    (0..frame.height())
        .flat_map(|y| (0..frame.width()).map(move |x| (x, y)))
        .filter_map(|(x, y)| frame.pixel(x, y))
        .map(|[r, g, b]| u32::from(r.max(g).max(b)))
        .sum()
}

fn the_box(look: KeyAppearance, side: u32) -> (u32, u32, u32, u32) {
    ink_box(&render(&face("Deploy", look), side, side), BG).expect("ink")
}

#[test]
fn a_hidden_title_draws_nothing_and_keeps_its_text() {
    let hidden = face(
        "Deploy",
        KeyAppearance {
            show_title: Some(false),
            bold: Some(true),
            ..KeyAppearance::default()
        },
    );
    assert_eq!(ink_box(&render(&hidden, 72, 72), BG), None);
    // The title and its formatting are still there to show again.
    assert_eq!(hidden.title, "Deploy");
    assert_eq!(hidden.appearance.as_ref().unwrap().bold, Some(true));
    let shown = face("Deploy", KeyAppearance::default());
    assert!(ink_box(&render(&shown, 72, 72), BG).is_some());
}

#[test]
fn top_middle_and_bottom_stay_inside_the_key() {
    for side in [72, 96, 120] {
        let at = |align| {
            the_box(
                KeyAppearance {
                    title_align: Some(align),
                    ..KeyAppearance::default()
                },
                side,
            )
        };
        let (top, middle, bottom) = (
            at(TitleAlign::Top),
            at(TitleAlign::Middle),
            at(TitleAlign::Bottom),
        );
        assert!(top.1 < middle.1 && middle.1 < bottom.1, "{side}");
        // Within the margins kept for the toggle mark and progress bar.
        let margin = (side as f32 / 12.0).max(3.0) as u32;
        assert!(top.1 >= margin - 1, "{side}: {top:?}");
        assert!(bottom.3 <= side - margin, "{side}: {bottom:?}");
    }
}

#[test]
fn bold_is_heavier_italic_leans_and_underline_adds_a_line() {
    let plain = render(&face("Deploy", KeyAppearance::default()), 72, 72);
    let bold = render(
        &face(
            "Deploy",
            KeyAppearance {
                bold: Some(true),
                ..KeyAppearance::default()
            },
        ),
        72,
        72,
    );
    assert!(ink(&bold) > ink(&plain) * 105 / 100);
    // Explicitly not bold is the plain title.
    let not_bold = KeyAppearance {
        bold: Some(false),
        ..KeyAppearance::default()
    };
    assert_eq!(render(&face("Deploy", not_bold), 72, 72).rgb(), plain.rgb());

    // Italic: the same rows, the top of the letters further right than
    // their foot.
    let lean = |frame: &Frame, title: &str| {
        let (_, top, _, bottom) = ink_box(frame, BG).unwrap();
        let first = |y: u32| (0..frame.width()).find(|x| frame.pixel(*x, y) != Some(BG));
        (first(top + 1), first(bottom.saturating_sub(1)), title.len())
    };
    let italic = render(
        &face(
            "IIII",
            KeyAppearance {
                italic: Some(true),
                ..KeyAppearance::default()
            },
        ),
        72,
        72,
    );
    let upright = render(&face("IIII", KeyAppearance::default()), 72, 72);
    let (Some(italic_top), Some(italic_foot), _) = lean(&italic, "IIII") else {
        panic!("italic ink")
    };
    let (Some(upright_top), Some(upright_foot), _) = lean(&upright, "IIII") else {
        panic!("upright ink")
    };
    assert!(italic_top > italic_foot, "{italic_top} {italic_foot}");
    assert!(upright_top.abs_diff(upright_foot) <= 1);

    // Underline: ink below the letters' baseline, as wide as the text.
    let underlined = render(
        &face(
            "mmmm",
            KeyAppearance {
                underline: Some(true),
                ..KeyAppearance::default()
            },
        ),
        72,
        72,
    );
    let plain_m = render(&face("mmmm", KeyAppearance::default()), 72, 72);
    let (l0, _, r0, b0) = ink_box(&plain_m, BG).unwrap();
    let (l1, _, r1, b1) = ink_box(&underlined, BG).unwrap();
    assert!(b1 > b0, "the line is below the letters: {b0} {b1}");
    assert!(l1.abs_diff(l0) <= 1 && r1.abs_diff(r0) <= 2);
}

#[test]
fn styled_titles_stay_inside_the_text_area_in_every_mode() {
    let long = "Deploy production servers tonight";
    for side in [72, 96, 120] {
        let pad = (side as f32 * 0.07).max(2.0);
        for mode in [
            TextMode::Wrap,
            TextMode::SingleLine,
            TextMode::AutoFit,
            TextMode::Marquee,
        ] {
            for (bold, italic, underline) in [(true, true, true), (false, true, false)] {
                let look = KeyAppearance {
                    bold: Some(bold),
                    italic: Some(italic),
                    underline: Some(underline),
                    text: Some(deckpilot_core::TextAppearance {
                        mode,
                        ..deckpilot_core::TextAppearance::default()
                    }),
                    ..KeyAppearance::default()
                };
                let styled = face(long, look);
                let frame = render(&styled, side, side);
                let (left, _, right, _) = ink_box(&frame, BG).unwrap();
                assert!(
                    left as f32 >= pad - 1.5 && right as f32 <= side as f32 - pad + 1.5,
                    "{mode:?} {side}: {left}..{right}"
                );
                // Scrolling frames stay inside too.
                if let Some(marquee) = marquee(&styled, side, side, &Icons::default()) {
                    let end = marquee.distance() as f32;
                    let scrolled = render_scrolled(
                        &styled,
                        side,
                        side,
                        &Icons::default(),
                        &DefaultAppearance::BUILTIN,
                        end,
                    );
                    let (left, _, right, _) = ink_box(&scrolled, BG).unwrap();
                    assert!(left as f32 >= pad - 1.5 && right as f32 <= side as f32 - pad + 1.5);
                }
            }
        }
    }
}

#[test]
fn auto_fit_keeps_the_style_while_it_shrinks() {
    let look = KeyAppearance {
        bold: Some(true),
        italic: Some(true),
        text: Some(deckpilot_core::TextAppearance {
            mode: TextMode::AutoFit,
            ..deckpilot_core::TextAppearance::default()
        }),
        ..KeyAppearance::default()
    };
    let styled = face("Deployment", look);
    let request_look = styled.appearance.clone().unwrap();
    let laid = layout(
        &Request {
            face: &styled,
            look: &request_look,
            colors: key_colors(&styled, &DefaultAppearance::BUILTIN),
            has_mark: false,
            over_icon: false,
        },
        72,
        72,
    );
    let line = &laid.lines[0];
    assert_eq!(line.style.weight, Weight::SemiBold);
    assert!(line.style.italic);
    assert!(laid.report.unwrap().fits);
}

#[test]
fn a_size_in_points_is_pixels_on_a_72_pixel_key() {
    let at = |size| {
        let (_, top, _, bottom) = the_box(
            KeyAppearance {
                text: Some(deckpilot_core::TextAppearance {
                    size: Some(size),
                    ..deckpilot_core::TextAppearance::default()
                }),
                ..KeyAppearance::default()
            },
            72,
        );
        bottom - top
    };
    assert!(at(20) > at(10) * 17 / 10);
}

#[test]
fn colour_is_the_models_default_until_the_key_sets_one() {
    let mk2 = models::stream_deck_mk2().effective_appearance();
    let plain = face("Deploy", KeyAppearance::default());
    let colors = key_colors(&plain, &mk2);
    assert_eq!(colors.text, [255, 255, 255]);
    assert_eq!(colors.background, [0, 0, 0]);
    let red = face(
        "Deploy",
        KeyAppearance {
            text_color: Some(Color([220, 38, 38])),
            ..KeyAppearance::default()
        },
    );
    assert_eq!(key_colors(&red, &mk2).text, [220, 38, 38]);
}

#[test]
fn every_title_font_has_faces_and_unknown_ones_draw_as_inter() {
    for font in TITLE_FONTS {
        assert!(crate::has_title_font(font.id), "{}", font.id);
    }
    let unknown = KeyAppearance {
        font_family: Some(FontFamilyId::new("someday-sans")),
        ..KeyAppearance::default()
    };
    let inter = KeyAppearance {
        font_family: Some(FontFamilyId::new("inter")),
        ..KeyAppearance::default()
    };
    let plain = render(&face("Deploy", KeyAppearance::default()), 72, 72);
    assert_eq!(render(&face("Deploy", unknown), 72, 72).rgb(), plain.rgb());
    assert_eq!(render(&face("Deploy", inter), 72, 72).rgb(), plain.rgb());
}

#[test]
fn a_look_stored_before_typography_renders_as_before() {
    // The fields absent, and every one of them explicitly "off".
    let old = KeyAppearance {
        title_align: Some(TitleAlign::Top),
        text_color: Some(Color([255, 204, 0])),
        ..KeyAppearance::default()
    };
    let mut off = old.clone();
    off.bold = Some(false);
    off.italic = Some(false);
    off.underline = Some(false);
    assert_eq!(
        render(&face("Deploy", old), 72, 72).rgb(),
        render(&face("Deploy", off), 72, 72).rgb()
    );
}
