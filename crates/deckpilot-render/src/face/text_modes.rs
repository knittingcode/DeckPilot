//! The text modes of `docs/key-text.md`, through the renderer.

use deckpilot_core::{ButtonState, FaceStyle, StateSnapshot, render_label};

use super::tests::ink_box;
use super::*;

const BG: [u8; 3] = [0, 0, 0];

fn face(title: &str, text: TextAppearance) -> KeyFace {
    KeyFace::new(FaceStyle::Action, title).with_appearance(Some(&KeyAppearance {
        text: Some(text),
        ..KeyAppearance::default()
    }))
}

fn mode(mode: TextMode) -> TextAppearance {
    TextAppearance {
        mode,
        ..TextAppearance::default()
    }
}

fn report(face: &KeyFace, side: u32) -> TextReport {
    text_report(face, side, side, &Icons::default()).expect("a title")
}

fn placed(face: &KeyFace, width: u32, height: u32) -> Layout {
    let look = face.appearance.clone().unwrap_or_default();
    layout(
        &Request {
            face,
            look: &look,
            colors: key_colors(face, &DefaultAppearance::BUILTIN),
            has_mark: face.symbol.is_some(),
            over_icon: false,
        },
        width,
        height,
    )
}

fn titles(face: &KeyFace, side: u32) -> Vec<String> {
    placed(face, side, side)
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect()
}

/// The text area of a key: inside the padding.
fn area(side: u32) -> (f32, f32) {
    let pad = (side as f32 * 0.07).max(2.0);
    (pad, side as f32 - pad)
}

/// Lines that are not clipped are no wider than the text area and drawn
/// inside it; clipped ones draw only inside it.
fn assert_inside(face: &KeyFace, side: u32) {
    let (from, to) = area(side);
    for line in placed(face, side, side).lines {
        if let Some((left, right)) = line.clip {
            assert!(left >= from - 0.01 && right <= to + 0.01);
        } else {
            let width = line.style.width(&line.text);
            assert!(
                line.left >= from - 0.5 && line.left + width <= to + 0.5,
                "{:?} at {} wide {width} on {side}",
                line.text,
                line.left
            );
        }
    }
    let (left, _, right, _) = ink_box(&render(face, side, side), BG).unwrap();
    // Anti-aliasing may reach one pixel further.
    assert!(
        left as f32 >= from - 1.5 && right as f32 <= to + 1.5,
        "{left}..{right}"
    );
}

#[test]
fn the_default_is_wrapping_as_before() {
    let plain = KeyFace::new(FaceStyle::Action, "Open the project dashboard");
    let wrapped = face("Open the project dashboard", TextAppearance::default());
    assert_eq!(
        wrapped.appearance, None,
        "an empty text appearance is not kept"
    );
    let explicit = KeyFace {
        appearance: Some(KeyAppearance {
            text: Some(mode(TextMode::Wrap)),
            background: Some(deckpilot_core::Color(BG).into()),
            ..KeyAppearance::default()
        }),
        ..plain.clone()
    };
    let coloured = KeyFace {
        appearance: Some(KeyAppearance {
            background: Some(deckpilot_core::Color(BG).into()),
            ..KeyAppearance::default()
        }),
        ..plain.clone()
    };
    assert_eq!(
        render(&explicit, 72, 72).rgb(),
        render(&coloured, 72, 72).rgb()
    );
    let wrapped = report(&plain, 72);
    assert_eq!(wrapped.mode, TextMode::Wrap);
    assert_eq!(wrapped.lines, 3);
    assert!(wrapped.fits && !wrapped.truncated && !wrapped.needs_scroll);
    // Too long for three lines: cut, and reported.
    let long = report(
        &KeyFace::new(FaceStyle::Action, "Open the project dashboard now please"),
        72,
    );
    assert!(long.truncated && !long.fits);
    assert!(
        (wrapped.font_points - 11.16).abs() < 0.01,
        "{}",
        wrapped.font_points
    );
}

#[test]
fn wrap_keeps_explicit_line_breaks_and_limits_lines() {
    let text = mode(TextMode::Wrap);
    assert_eq!(
        titles(&face("Build\nDeploy", text), 72),
        ["Build", "Deploy"]
    );
    // Short lines stay apart, words are not broken.
    assert_eq!(titles(&face("A\nB C", text), 72), ["A", "B C"]);
    // A long word alone is broken; words that fit never are.
    let lines = titles(&face("Supercalifragilistic", text), 72);
    assert!(
        lines.len() > 1 && lines.concat() == "Supercalifragilistic",
        "{lines:?}"
    );
    let two = TextAppearance {
        max_lines: Some(2),
        ..text
    };
    let long = face("Open the project dashboard in the browser", two);
    let lines = titles(&long, 72);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].ends_with('…'), "{lines:?}");
    let cut = report(&long, 72);
    assert!(cut.truncated && !cut.fits);
    // One line asked for: the line breaks read as spaces there.
    let one = TextAppearance {
        max_lines: Some(1),
        ..text
    };
    assert_eq!(titles(&face("Go\nNow", one), 72), ["Go Now"]);
    // More lines than the key has room for: as many as fit, never over
    // the edge.
    let six = TextAppearance {
        max_lines: Some(6),
        size: Some(14),
        ..text
    };
    let crowded = face("one two three four five six seven eight nine ten", six);
    let lines = titles(&crowded, 72);
    assert!(lines.len() < 6, "{lines:?}");
    assert!(lines.last().unwrap().ends_with('…'));
    for side in [72, 80, 96, 120] {
        assert_inside(&long, side);
        assert_inside(&crowded, side);
    }
}

#[test]
fn single_line_keeps_its_size_and_cuts_or_clips() {
    let text = TextAppearance {
        mode: TextMode::SingleLine,
        size: Some(12),
        ..TextAppearance::default()
    };
    let long = face("Open the project dashboard", text);
    let lines = titles(&long, 72);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("Open") && lines[0].ends_with('…'),
        "{lines:?}"
    );
    let cut = report(&long, 72);
    assert!((cut.font_points - 12.0).abs() < 0.01, "never shrunk");
    assert!(cut.truncated && !cut.fits && cut.lines == 1);
    assert_inside(&long, 72);

    let clip = face(
        "Open the project dashboard",
        TextAppearance {
            overflow: Some(TextOverflow::Clip),
            ..text
        },
    );
    let placed = placed(&clip, 72, 72);
    assert_eq!(placed.lines[0].text, "Open the project dashboard");
    assert!(placed.lines[0].clip.is_some());
    assert!(report(&clip, 72).truncated);
    assert_inside(&clip, 72);
    // A line break is a space on one line.
    assert_eq!(titles(&face("Go\nNow", text), 72), ["Go Now"]);
    // What fits is neither cut nor clipped.
    let short = face("Mail", text);
    assert!(report(&short, 72).fits);
    assert_eq!(titles(&short, 72), ["Mail"]);
}

#[test]
fn auto_fit_shrinks_only_as_needed_and_reports_what_does_not_fit() {
    let text = TextAppearance {
        mode: TextMode::AutoFit,
        size: Some(14),
        min_size: Some(8),
        ..TextAppearance::default()
    };
    // Short: the preferred size.
    let short = report(&face("Mail", text), 72);
    assert!((short.font_points - 14.0).abs() < 0.01 && short.fits);
    // Never larger than preferred.
    let small = report(
        &face(
            "Hi",
            TextAppearance {
                size: Some(8),
                min_size: Some(6),
                ..text
            },
        ),
        72,
    );
    assert!((small.font_points - 8.0).abs() < 0.01);
    // Longer: smaller, down to what fits, on one line.
    let longer = face("Deployment", text);
    let fitted = report(&longer, 72);
    assert!(fitted.fits && !fitted.truncated);
    assert!(
        fitted.font_points < 14.0 && fitted.font_points >= 8.0,
        "{fitted:?}"
    );
    assert_eq!(titles(&longer, 72), ["Deployment"]);
    assert_inside(&longer, 72);
    // As large as it can be: a little larger would not fit.
    let line = Style::new(Weight::Medium, fitted.font_pixels * 1.03);
    let (from, to) = area(72);
    assert!(line.width("Deployment") > to - from);
    // Too long even at the minimum: the minimum, cut, and reported.
    let long = face("Supercalifragilisticexpialidocious", text);
    let failed = report(&long, 72);
    assert!(!failed.fits && failed.truncated);
    assert!((failed.font_points - 8.0).abs() < 0.01, "{failed:?}");
    let lines = titles(&long, 72);
    assert!(lines[0].ends_with('…'), "{lines:?}");
    assert_inside(&long, 72);
}

#[test]
fn multiline_auto_fit_wraps_before_it_shrinks() {
    let text = TextAppearance {
        mode: TextMode::AutoFit,
        fit: Some(FitLines::Multiline),
        size: Some(13),
        min_size: Some(7),
        ..TextAppearance::default()
    };
    let wrapped = face("Open project dashboard", text);
    let fitted = report(&wrapped, 72);
    assert!(fitted.fits && fitted.lines >= 2, "{fitted:?}");
    let lines = titles(&wrapped, 72);
    for word in ["Open", "project", "dashboard"] {
        assert!(lines.iter().any(|line| line.contains(word)), "{lines:?}");
    }
    assert_inside(&wrapped, 72);
    // A single line fit would have to be much smaller.
    let single = report(
        &face(
            "Open project dashboard",
            TextAppearance {
                fit: Some(FitLines::SingleLine),
                ..text
            },
        ),
        72,
    );
    assert!(
        single.font_points < fitted.font_points,
        "{single:?} {fitted:?}"
    );
    // At most the lines asked for.
    let two = TextAppearance {
        max_lines: Some(2),
        ..text
    };
    let many = report(&face("one two three four five six seven", two), 72);
    assert!(many.lines <= 2);
    // Not even at the minimum: reported, never drawn past the key.
    let impossible = face(
        "a very long title that cannot possibly fit on one small key at all",
        TextAppearance {
            min_size: Some(12),
            ..two
        },
    );
    let failed = report(&impossible, 72);
    assert!(!failed.fits && failed.truncated);
    assert!((failed.font_points - 12.0).abs() < 0.01);
    assert_inside(&impossible, 72);
}

#[test]
fn marquee_scrolls_only_what_is_too_wide() {
    let text = TextAppearance {
        mode: TextMode::Marquee,
        speed: Some(40),
        pause: Some(500),
        ..TextAppearance::default()
    };
    let icons = Icons::default();
    // Fits: no scrolling, no animation.
    let short = face("Mail", text);
    assert!(!report(&short, 72).needs_scroll);
    assert_eq!(marquee(&short, 72, 72, &icons), None);
    assert_eq!(
        render_scrolled(&short, 72, 72, &icons, &DefaultAppearance::BUILTIN, 10.0).rgb(),
        render(&short, 72, 72).rgb()
    );

    let long = face("Deploy the staging environment", text);
    let scrolling = report(&long, 72);
    assert!(scrolling.needs_scroll && scrolling.fits && scrolling.lines == 1);
    let marquee = marquee(&long, 72, 72, &icons).expect("scrolls");
    assert_eq!(marquee.distance(), scrolling.scroll_distance.ceil() as u32);
    assert!((marquee.speed() - 40.0).abs() < f32::EPSILON);
    assert_eq!(marquee.pause(), std::time::Duration::from_millis(500));
    // Still (reduced motion, the app's preview): cut with `…`.
    let still = titles(&long, 72);
    assert!(still[0].ends_with('…'));
    // Moving: the whole line, clipped to the text area.
    let start = render_scrolled(&long, 72, 72, &icons, &DefaultAppearance::BUILTIN, 0.0);
    let middle = render_scrolled(&long, 72, 72, &icons, &DefaultAppearance::BUILTIN, 20.0);
    let end = render_scrolled(
        &long,
        72,
        72,
        &icons,
        &DefaultAppearance::BUILTIN,
        marquee.distance() as f32,
    );
    let past = render_scrolled(&long, 72, 72, &icons, &DefaultAppearance::BUILTIN, 999.0);
    assert_ne!(start.rgb(), middle.rgb());
    assert_ne!(middle.rgb(), end.rgb());
    assert_eq!(end.rgb(), past.rgb(), "never past the end");
    assert_ne!(start.rgb(), render(&long, 72, 72).rgb());
    let (from, to) = area(72);
    for frame in [&start, &middle, &end] {
        let (left, _, right, _) = ink_box(frame, BG).unwrap();
        assert!(left as f32 >= from - 1.0 && right as f32 <= to + 1.0);
    }
    // The same frame for the same offset: unchanged frames can be skipped.
    assert_eq!(
        render_scrolled(&long, 72, 72, &icons, &DefaultAppearance::BUILTIN, 20.0).rgb(),
        middle.rgb()
    );
}

#[test]
fn alignment_moves_the_lines() {
    let at = |align: TextAlign| {
        let face = face(
            "Mail",
            TextAppearance {
                align: Some(align),
                ..TextAppearance::default()
            },
        );
        let (left, _, right, _) = ink_box(&render(&face, 72, 72), BG).unwrap();
        (left, right)
    };
    let (left, center, right) = (
        at(TextAlign::Left),
        at(TextAlign::Center),
        at(TextAlign::Right),
    );
    assert!(left.0 < center.0 && center.0 < right.0);
    let (from, to) = area(72);
    assert!((left.0 as f32 - from).abs() <= 2.0, "{left:?}");
    assert!((to - right.1 as f32).abs() <= 2.0, "{right:?}");
}

/// Texts are cut between characters as a reader sees them: never inside a
/// Polish letter, an emoji sequence or a letter with a combining mark.
#[test]
fn unicode_is_never_cut_inside_a_character() {
    let polish = "Zażółć gęślą jaźń ĄĆĘŁŃÓŚŹŻ ąćęłńóśźż";
    let emoji = "Deploy 👨‍👩‍👧‍👦👍🏽🇵🇱 now";
    let marks = "Cafe\u{301} cre\u{300}me bru\u{302}le\u{301}e";
    for title in [polish, emoji, marks] {
        for text in [
            mode(TextMode::SingleLine),
            TextAppearance {
                max_lines: Some(1),
                ..mode(TextMode::Wrap)
            },
            TextAppearance {
                mode: TextMode::AutoFit,
                min_size: Some(10),
                size: Some(10),
                ..TextAppearance::default()
            },
            mode(TextMode::Marquee),
        ] {
            for side in [72, 96] {
                let face = face(title, text);
                for line in titles(&face, side) {
                    let kept = line.trim_end_matches('…');
                    let flat = title.split_whitespace().collect::<Vec<_>>().join(" ");
                    assert!(flat.starts_with(kept), "{line:?} of {title:?}");
                    let boundaries: Vec<usize> =
                        unicode_segmentation::UnicodeSegmentation::grapheme_indices(
                            flat.as_str(),
                            true,
                        )
                        .map(|(at, _)| at)
                        .chain(std::iter::once(flat.len()))
                        .collect();
                    assert!(
                        boundaries.contains(&kept.len()),
                        "{line:?} cuts a character of {title:?}"
                    );
                }
                assert_inside(&face, side);
            }
        }
    }
    // Polish letters are drawn, not replaced.
    let wrapped = face(polish, mode(TextMode::Wrap));
    assert!(
        titles(&wrapped, 120).concat().starts_with("Zażółćgęślą"),
        "{:?}",
        titles(&wrapped, 120)
    );
}

/// Every key resolution of the device catalogue lays a title out alike:
/// the same lines at the same size in points, so the 72 px preview of a
/// 120 px key does not wrap differently, and a preview drawn larger than
/// the device (the app's sharp previews) wraps as the device does.
#[test]
fn every_key_resolution_lays_out_alike() {
    let catalog = deckpilot_device::ModelCatalog::builtin();
    let mut sides: Vec<(u32, u32)> = catalog
        .iter()
        .filter_map(|model| model.capabilities.keys.and_then(|k| k.image))
        .map(|image| (image.width, image.height))
        .collect();
    // The Stream Deck Mini's 80 px keys, not in the catalogue.
    sides.push((80, 80));
    sides.sort_unstable();
    sides.dedup();
    assert!(sides.contains(&(72, 72)) && sides.contains(&(96, 96)) && sides.contains(&(120, 120)));
    for text in [
        mode(TextMode::Wrap),
        TextAppearance {
            max_lines: Some(2),
            ..mode(TextMode::Wrap)
        },
        mode(TextMode::SingleLine),
        mode(TextMode::AutoFit),
        TextAppearance {
            fit: Some(FitLines::Multiline),
            ..mode(TextMode::AutoFit)
        },
        mode(TextMode::Marquee),
    ] {
        for title in [
            "Mail",
            "Open the project dashboard",
            "Supercalifragilistic",
            "Zażółć gęślą",
        ] {
            let face = face(title, text);
            let reference = titles(&face, 72);
            let points = report(&face, 72).font_points;
            for &(width, height) in &sides {
                assert_eq!(titles(&face, width), reference, "{title} {text:?} {width}");
                let at = text_report(&face, width, height, &Icons::default()).unwrap();
                assert!((at.font_points - points).abs() < 0.3, "{title} {width}");
                assert_inside(&face, width);
                // Twice as sharp, the same lines.
                let lines: Vec<String> = placed(&face, width * 2, height * 2)
                    .lines
                    .into_iter()
                    .map(|line| line.text)
                    .collect();
                assert_eq!(lines, reference, "{title} {text:?} {width} x2");
            }
        }
    }
    // A touch-strip zone (200 x 100): laid out by its shorter side.
    let zone = face("Volume", mode(TextMode::AutoFit));
    assert!(
        text_report(&zone, 200, 100, &Icons::default())
            .unwrap()
            .fits
    );
}

/// Dynamic labels are resolved before the title is laid out, so a new
/// value (another project, a plugin's label) is fitted again.
#[test]
fn resolved_labels_are_fitted_again_when_they_change() {
    let text = TextAppearance {
        mode: TextMode::AutoFit,
        size: Some(14),
        min_size: Some(7),
        ..TextAppearance::default()
    };
    let shop =
        deckpilot_core::Project::new(deckpilot_core::ProjectKey::new("SHOP").unwrap(), "Shop");
    let long = deckpilot_core::Project::new(
        deckpilot_core::ProjectKey::new("WAREHOUSE").unwrap(),
        "Warehouse",
    );
    let label = |project| {
        face(
            &render_label("Git {project.key}", "Work", Some(project)),
            text,
        )
    };
    let short = report(&label(&shop), 72);
    let longer = report(&label(&long), 72);
    assert!(short.fits && longer.fits);
    assert!(
        longer.font_points < short.font_points,
        "{short:?} {longer:?}"
    );
    let mut cache = FrameCache::new(8);
    cache.frame(
        &label(&shop),
        72,
        72,
        &Icons::default(),
        &DefaultAppearance::BUILTIN,
    );
    cache.frame(
        &label(&long),
        72,
        72,
        &Icons::default(),
        &DefaultAppearance::BUILTIN,
    );
    assert_eq!(cache.rendered(), 2, "a new label is drawn again");

    // A plugin's (or module's) label: the binding's text settings apply.
    let state = ButtonState::new().with_label("Pipeline running for main");
    let plugin = face("Pipeline", text).with_snapshot(&StateSnapshot::Ready { state });
    assert_eq!(plugin.title, "Pipeline running for main");
    let fitted = report(&plugin, 72);
    assert_eq!(fitted.mode, TextMode::AutoFit);
    assert!(fitted.font_points < 14.0);
    assert_inside(&plugin, 72);
}

#[test]
fn hidden_titles_and_empty_faces_have_no_report() {
    let icons = Icons::default();
    assert_eq!(text_report(&KeyFace::empty(), 72, 72, &icons), None);
    let hidden = KeyFace::new(FaceStyle::Action, "Mail").with_appearance(Some(&KeyAppearance {
        show_title: Some(false),
        ..KeyAppearance::default()
    }));
    assert_eq!(text_report(&hidden, 72, 72, &icons), None);
    let untitled = report(&KeyFace::new(FaceStyle::Action, ""), 72);
    assert!(untitled.fits && untitled.lines == 0);
}

/// The cost of a scrolling key, as `docs/key-text.md` reports it: frames
/// a second, render time per frame, and the bytes a second the device's
/// driver sends (JPEG at quality 90, as `elgato-streamdeck` encodes key
/// images). Run with `--nocapture` (and `--release` for the documented
/// numbers) to see them.
#[test]
fn the_cost_of_a_scrolling_key_is_bounded() {
    let text = TextAppearance {
        mode: TextMode::Marquee,
        ..TextAppearance::default()
    };
    let icons = Icons::default();
    for side in [72_u32, 96, 120] {
        let face =
            face("Deploy the staging environment to the cloud", text).with_symbol(Some("rocket"));
        let marquee = marquee(&face, side, side, &icons).expect("scrolls");
        let timeline = crate::marquee_timeline(&marquee, crate::MARQUEE_FPS);
        let cycle = marquee.cycle().as_secs_f64();
        let started = std::time::Instant::now();
        let mut bytes = 0;
        for (offset, _) in &timeline {
            let frame = render_scrolled(
                &face,
                side,
                side,
                &icons,
                &DefaultAppearance::BUILTIN,
                *offset as f32,
            );
            let image =
                image::RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec())
                    .unwrap();
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
                .encode_image(&image)
                .unwrap();
            bytes += jpeg.len();
        }
        let per_frame = started.elapsed().as_secs_f64() / timeline.len() as f64;
        let fps = timeline.len() as f64 / cycle;
        let rate = bytes as f64 / cycle;
        eprintln!(
            "{side} px: {} frames in a {cycle:.2} s cycle = {fps:.1} frames/s, \
             {:.2} ms per frame (render + JPEG), {:.1} KB/s",
            timeline.len(),
            per_frame * 1000.0,
            rate / 1024.0
        );
        assert!(fps <= f64::from(crate::MARQUEE_FPS) + 0.01, "{fps}");
        // Generous for unoptimized builds; see docs/key-text.md for the
        // release numbers.
        assert!(per_frame < 0.05, "{per_frame}");
        assert!(rate < 200.0 * 1024.0, "{rate}");
    }
}
