use super::*;
use deckpilot_core::{Accent, Color, FaceStyle};

/// What a key without a background shows on a Stream Deck: the black
/// display behind its transparent default.
const BLACK: [u8; 3] = [0, 0, 0];
/// The value line on a Stream Deck: white a fifth of the way to black.
const MUTED: [u8; 3] = [204, 204, 204];

fn lit(frame: &Frame, background: [u8; 3]) -> usize {
    (0..frame.height())
        .flat_map(|y| (0..frame.width()).map(move |x| (x, y)))
        .filter(|(x, y)| frame.pixel(*x, *y) != Some(background))
        .count()
}

/// Whether `pixel` is (anti-aliased) ink of `color`: every channel within
/// 64 of it, as text pixels covered for the most part are.
fn near(pixel: Option<[u8; 3]>, color: [u8; 3]) -> bool {
    pixel.is_some_and(|p| p.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 64))
}

fn rows_with(frame: &Frame, color: [u8; 3]) -> Vec<u32> {
    (0..frame.height())
        .filter(|y| (0..frame.width()).any(|x| near(frame.pixel(x, *y), color)))
        .collect()
}

fn count(frame: &Frame, color: [u8; 3]) -> usize {
    (0..frame.height())
        .flat_map(|y| (0..frame.width()).map(move |x| (x, y)))
        .filter(|(x, y)| frame.pixel(*x, *y) == Some(color))
        .count()
}

fn appearance() -> KeyAppearance {
    KeyAppearance::default()
}

/// Columns and rows that differ from the background, as a box.
pub(super) fn ink_box(frame: &Frame, background: [u8; 3]) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            if frame.pixel(x, y) != Some(background) {
                found = Some(found.map_or((x, y, x, y), |(a, b, c, d)| {
                    (a.min(x), b.min(y), c.max(x), d.max(y))
                }));
            }
        }
    }
    found
}

#[test]
fn empty_faces_are_black() {
    let frame = render(&KeyFace::empty(), 72, 72);
    assert_eq!(lit(&frame, [0, 0, 0]), 0);
    let mut marked = KeyFace::empty();
    marked.symbol = Some("cpu".into());
    assert_eq!(lit(&render(&marked, 72, 72), [0, 0, 0]), 0);
}

#[test]
fn text_is_drawn_within_bounds_for_any_size() {
    let face = KeyFace::new(FaceStyle::Action, "Start Work").with_detail("SHOP");
    for (w, h) in [
        (72, 72),
        (80, 80),
        (96, 96),
        (120, 120),
        (200, 100),
        (8, 8),
        (0, 0),
    ] {
        let frame = render(&face, w, h);
        assert_eq!((frame.width(), frame.height()), (w, h));
        if w >= 72 {
            assert!(lit(&frame, BLACK) > 20);
        }
    }
}

/// The text is a real font: anti-aliased, proportional, and as large on
/// every key relative to its size.
#[test]
fn titles_are_anti_aliased_and_scale_with_the_key() {
    let face = KeyFace::new(FaceStyle::Action, "Mail");
    let bg = BLACK;
    let frame = render(&face, 72, 72);
    let shades: std::collections::BTreeSet<[u8; 3]> = (0..72)
        .flat_map(|y| (0..72).map(move |x| (x, y)))
        .filter_map(|(x, y)| frame.pixel(x, y))
        .collect();
    assert!(shades.len() > 12, "anti-aliased: {}", shades.len());
    // Readable on the smallest key: the title's letters are at least 8
    // pixels tall (an em of 11 px).
    let (_, top, _, bottom) = ink_box(&frame, bg).unwrap();
    assert!(bottom - top >= 7, "{top}..{bottom}");
    // The same share of the key on a 96 and a 120 px key.
    let height = |side: u32| {
        let (_, top, _, bottom) = ink_box(&render(&face, side, side), bg).unwrap();
        f64::from(bottom - top) / f64::from(side)
    };
    for side in [96, 120] {
        assert!((height(side) - height(72)).abs() < 0.04, "{side}");
    }
    assert!(title_size(FontSize::Medium, 72.0) >= 11.0);
}

#[test]
fn polish_labels_keep_their_letters() {
    let key = |title: &str, detail: &str| {
        render(
            &KeyFace::new(FaceStyle::Action, title).with_detail(detail),
            72,
            72,
        )
        .rgb()
        .to_vec()
    };
    // Diacritics are drawn, not replaced or dropped.
    assert_ne!(key("Mgła", "12°C"), key("Mgla", "12 C"));
    assert_ne!(key("Mgła", "12°C"), key("Mg?a", "12?C"));
    assert_ne!(key("Żółć", ""), key("Zolc", ""));
    assert_ne!(key("ŻÓŁĆ", ""), key("ZOLC", ""));
    for word in ["Zażółć", "gęślą", "jaźń", "ĄĆĘŁŃÓŚŹŻ", "ąćęłńóśźż"] {
        assert_ne!(
            key(word, ""),
            key(&"?".repeat(word.chars().count()), ""),
            "{word}"
        );
    }
    // Letters are as wide as they are, not one cell each.
    let title = |text: &str| render(&KeyFace::new(FaceStyle::Action, text), 72, 72);
    let ink = |frame: &Frame| {
        (0..frame.width())
            .filter(|x| (0..frame.height()).any(|y| frame.pixel(*x, y) != frame.pixel(0, 0)))
            .count()
    };
    assert!(ink(&title("Łódź")) < ink(&title("WWWW")));
    assert!(ink(&title("iiii")) < ink(&title("mmmm")));
    // What the font lacks is a question mark, as before.
    assert_eq!(key("中", ""), key("?", ""));
    // Stored titles are drawn as they are: the face is not changed.
    let face = KeyFace::new(FaceStyle::Action, "Zażółć gęślą jaźń");
    let copy = face.clone();
    render(&face, 72, 72);
    assert_eq!(face, copy);
}

#[test]
fn long_titles_wrap_and_end_with_an_ellipsis() {
    let short = render(&KeyFace::new(FaceStyle::Action, "Deploy"), 72, 72);
    let long = render(
        &KeyFace::new(
            FaceStyle::Action,
            "Deploy the online shop to production and staging",
        ),
        72,
        72,
    );
    let bg = BLACK;
    let (_, top, _, bottom) = ink_box(&long, bg).unwrap();
    let (_, short_top, _, short_bottom) = ink_box(&short, bg).unwrap();
    // Several lines, all on the key, none touching its edges.
    assert!(bottom - top > 2 * (short_bottom - short_top));
    let (left, _, right, _) = ink_box(&long, bg).unwrap();
    assert!(left >= 2 && right <= 69, "{left}..{right}");
    assert!(top >= 2 && bottom <= 69);
}

#[test]
fn highlight_draws_a_border() {
    let face = KeyFace::new(FaceStyle::Action, "CMS").highlighted(true);
    let frame = render(&face, 72, 72);
    assert_eq!(frame.pixel(0, 0), Some(ACCENT));
    assert_eq!(frame.pixel(71, 71), Some(ACCENT));
    assert_eq!(frame.pixel(2, 36), Some(BLACK));
}

/// Icon first: the key's icon above the title, the value line under it.
#[test]
fn symbols_are_drawn_above_the_title_and_values_below_it() {
    let bg = BLACK;
    let plain = KeyFace::new(FaceStyle::Action, "CPU usage");
    let iconed = plain.clone().with_symbol(Some("cpu"));
    let frame = render(&iconed, 72, 72);
    let rows = rows_with(&frame, WHITE);
    // The icon starts in the upper part; the title is at the bottom.
    assert!(rows[0] < 16, "{rows:?}");
    let base_rows = rows_with(&render(&plain, 72, 72), WHITE);
    assert!(rows.last().unwrap() > base_rows.last().unwrap());
    assert!(lit(&frame, bg) > lit(&render(&plain, 72, 72), bg) + 80);
    // A different icon is a different key; an unknown name is the
    // fallback icon, as in the app.
    let other = render(&plain.clone().with_symbol(Some("hard-drive")), 72, 72);
    assert_ne!(other.rgb(), frame.rgb());
    assert_eq!(
        render(&plain.clone().with_symbol(Some("no-such-icon")), 72, 72).rgb(),
        render(
            &plain.clone().with_symbol(Some(text::FALLBACK_ICON)),
            72,
            72
        )
        .rgb()
    );
    // A live value goes under the title, the icon stays.
    let live = iconed.clone().with_detail("23%");
    let with_value = render(&live, 72, 72);
    let value_rows = rows_with(&with_value, MUTED);
    assert_ne!(value_rows, Vec::<u32>::new());
    assert!(*value_rows.last().unwrap() > rows.last().unwrap().saturating_sub(4));
    assert!(
        rows_with(&with_value, WHITE)[0] < 16,
        "the icon is still drawn"
    );
    // Two value lines (e.g. received and sent).
    let two = render(
        &plain
            .clone()
            .with_symbol(Some("network"))
            .with_detail("↓ 2.1 MB/s\n↑ 340 kB/s"),
        72,
        72,
    );
    let one = render(
        &plain
            .clone()
            .with_symbol(Some("network"))
            .with_detail("↓ 2.1 MB/s"),
        72,
        72,
    );
    assert_ne!(two.rgb(), one.rgb());
    let look = KeyAppearance::default();
    let face = plain
        .clone()
        .with_symbol(Some("network"))
        .with_detail("↓ 2.1 MB/s\n↑ 340 kB/s");
    let placed = layout(
        &Request {
            face: &face,
            look: &look,
            colors: key_colors(&face, &DefaultAppearance::BUILTIN),
            has_mark: true,
            over_icon: false,
        },
        72,
        72,
    );
    let texts: Vec<&str> = placed.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert_eq!(&texts[1..], ["↓ 2.1 MB/s", "↑ 340 kB/s"]);
    let (_, mark_top, mark_side) = placed.mark.expect("the icon stays");
    assert!(mark_top + mark_side <= placed.block.unwrap().0);
    // A value too wide for the key is set a little smaller before it is
    // cut.
    let memory = plain.clone().with_detail("12.4/32 GiB");
    let placed = layout(
        &Request {
            face: &memory,
            look: &look,
            colors: key_colors(&memory, &DefaultAppearance::BUILTIN),
            has_mark: false,
            over_icon: false,
        },
        72,
        72,
    );
    assert_eq!(placed.lines.last().unwrap().text, "12.4/32 GiB");
    // Without a title, the icon is larger and centred.
    let bare = iconed.clone().with_appearance(Some(&KeyAppearance {
        show_title: Some(false),
        ..appearance()
    }));
    let (left, top, right, bottom) = ink_box(&render(&bare, 72, 72), bg).unwrap();
    // Larger than an icon above a title can be (at most 42 % of the key).
    let above_title = (72.0 * MARK_WITH_TEXT) as u32;
    assert!(
        bottom - top > above_title && right - left > above_title,
        "{left},{top} {right},{bottom}"
    );
    assert!((i64::from(top + bottom) / 2 - 36).abs() <= 3);
}

#[test]
fn geometry_of_every_model_keeps_marks_and_text_on_the_key() {
    let face = KeyFace::new(FaceStyle::Action, "Memory usage")
        .with_symbol(Some("memory-stick"))
        .with_detail("12.4 / 32 GiB")
        .with_indicator(Indicator::Warning)
        .with_progress(Some(39));
    let bg = BLACK;
    // Mini 80, MK.2 72, XL 96, Plus 120, the Plus touch strip's zones 200
    // x 100, Neo's info screen 248 x 58.
    for (w, h) in [
        (72, 72),
        (80, 80),
        (96, 96),
        (120, 120),
        (200, 100),
        (248, 58),
    ] {
        let frame = render(&face, w, h);
        let (left, top, right, bottom) = ink_box(&frame, bg).unwrap();
        assert!(left < w / 4 && right > w * 3 / 4, "{w}x{h}");
        assert!(top < h / 4 && bottom == h - 1, "{w}x{h}: {top}..{bottom}");
        assert!(!rows_with(&frame, MUTED).is_empty(), "{w}x{h}: value drawn");
    }
}

/// Accent, glyph, badge and series: drawn from the typed face at any
/// size, and never over what the user chose for the key.
#[test]
fn accent_glyph_badge_and_series_are_drawn_and_the_users_look_wins() {
    use deckpilot_core::Glyph;
    let plain = KeyFace::new(FaceStyle::Action, "Focus");
    let base = render(&plain, 96, 96);

    // An accent is the background; the title stays light on it.
    let mut accented = plain.clone();
    accented.accent = Some(Accent::Green);
    let frame = render(&accented, 96, 96);
    let green = marks::accent_color(Accent::Green);
    assert_eq!(frame.pixel(1, 50), Some(green));
    assert_eq!(count(&frame, WHITE), count(&base, WHITE));
    assert_eq!(count(&frame, BLACK), 0);

    // A glyph is drawn in the upper part and moves the title down.
    let mut with_glyph = plain.clone();
    with_glyph.glyph = Some(Glyph::Timer);
    let frame = render(&with_glyph, 96, 96);
    assert!(count(&frame, WHITE) > count(&base, WHITE) + 100);
    assert!(rows_with(&frame, WHITE)[0] < 20);
    assert!(rows_with(&frame, WHITE).last().unwrap() > rows_with(&base, WHITE).last().unwrap());
    // The state's glyph wins over the key's own icon.
    let mut both = with_glyph.clone();
    both.symbol = Some("cpu".into());
    assert_eq!(render(&both, 96, 96).rgb(), frame.rgb());

    // A badge in the top left corner, below the toggle mark.
    let mut badged = plain.clone();
    badged.badge = Some("7".into());
    let frame = render(&badged, 96, 96);
    assert_ne!(frame.pixel(3, 3), base.pixel(3, 3));
    badged.toggle = Some(true);
    let frame = render(&badged, 96, 96);
    assert_eq!(frame.pixel(48, 0), Some(GREEN));
    assert_eq!(
        frame.pixel(3, 1),
        Some(GREEN),
        "the toggle mark is not covered"
    );

    // A series is a lighter line behind the title: the title's pixels
    // are all still there.
    let mut charted = plain.clone();
    charted.series = Some(vec![10, 80, 30, 100, 0]);
    let frame = render(&charted, 96, 96);
    let line = marks::lighten(BLACK, 35);
    assert!(count(&frame, line) > 60);
    assert_eq!(count(&frame, WHITE), count(&base, WHITE));

    // The user's background and icon win over accent, glyph and symbol;
    // the user's text colour is the glyph's.
    let mut users = with_glyph.clone();
    users.accent = Some(Accent::Red);
    users.appearance = Some(KeyAppearance {
        background: Some(Color([1, 2, 3]).into()),
        text_color: Some(Color([250, 250, 0])),
        ..KeyAppearance::default()
    });
    let frame = render(&users, 96, 96);
    assert_eq!(frame.pixel(1, 50), Some([1, 2, 3]));
    assert_eq!(count(&frame, marks::accent_color(Accent::Red)), 0);
    assert!(count(&frame, [250, 250, 0]) > 100);
    let icon = icons::import("Halves", &icons::tests::png(&icons::tests::halves(64, 64))).unwrap();
    let mut profile = deckpilot_core::Profile::new("P");
    let id = icon.id;
    profile.icons.push(icon);
    let mut iconed = with_glyph.clone();
    iconed.appearance = Some(KeyAppearance {
        icon: Some(deckpilot_core::IconRef {
            id: id.into(),
            fit: deckpilot_core::IconFit::Fill,
        }),
        ..KeyAppearance::default()
    });
    let with_icon = render_with(&iconed, 96, 96, &Icons::from_profile(&profile));
    iconed.glyph = None;
    let without_glyph = render_with(&iconed, 96, 96, &Icons::from_profile(&profile));
    assert_eq!(with_icon.rgb(), without_glyph.rgb());
    iconed.symbol = Some("cpu".into());
    let with_symbol = render_with(&iconed, 96, 96, &Icons::from_profile(&profile));
    assert_eq!(
        with_symbol.rgb(),
        without_glyph.rgb(),
        "the custom icon wins"
    );

    // Empty and tiny faces: nothing is drawn, nothing panics.
    let mut empty = KeyFace::empty();
    empty.accent = Some(Accent::Blue);
    empty.glyph = Some(Glyph::Star);
    empty.badge = Some("1".into());
    empty.series = Some(vec![1, 2]);
    assert_eq!(lit(&render(&empty, 72, 72), [0, 0, 0]), 0);
    let mut all = accented.clone();
    all.glyph = Some(Glyph::Heart);
    all.badge = Some("9999".into());
    all.series = Some(vec![100; 32]);
    all.progress = Some(40);
    all.toggle = Some(false);
    all.indicator = Some(Indicator::Running);
    all.symbol = Some("zap".into());
    for (w, h) in [(1, 1), (8, 8), (72, 72), (144, 144), (200, 100), (100, 30)] {
        let frame = render(&all, w, h);
        assert_eq!((frame.width(), frame.height()), (w, h));
    }
    // Faces differ by their marks, so the cache keeps them apart.
    let mut cache = FrameCache::new(4);
    let icons = Icons::default();
    assert_ne!(
        cache
            .frame(&accented, 72, 72, &icons, &DefaultAppearance::BUILTIN)
            .rgb(),
        cache
            .frame(&plain, 72, 72, &icons, &DefaultAppearance::BUILTIN)
            .rgb()
    );
    assert_eq!(cache.rendered(), 2);
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
        // The thin loading ring is anti-aliased all round: within a few
        // levels of its colour.
        let close = |pixel: Option<[u8; 3]>| {
            pixel.is_some_and(|p| p.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 4))
        };
        assert!(
            (0..72u32).any(|x| (0..20u32).any(|y| close(frame.pixel(x, y)))),
            "{indicator:?}"
        );
    }
    // Loading is a ring: its centre is not filled.
    let ring = render(&plain.clone().with_indicator(Indicator::Loading), 72, 72);
    let dot = render(&plain.clone().with_indicator(Indicator::Unknown), 72, 72);
    assert!(lit(&ring, BLACK) < lit(&dot, BLACK));
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
    let none = Icons::default();
    let a = KeyFace::new(FaceStyle::Action, "A");
    let b = KeyFace::new(FaceStyle::Action, "B");
    let c = KeyFace::new(FaceStyle::Action, "C");
    let first = cache.frame(&a, 72, 72, &none, &DefaultAppearance::BUILTIN);
    assert_eq!(
        cache.frame(&a, 72, 72, &none, &DefaultAppearance::BUILTIN),
        first
    );
    assert_eq!(cache.rendered(), 1);
    cache.frame(&a, 96, 96, &none, &DefaultAppearance::BUILTIN);
    assert_eq!(cache.rendered(), 2);
    // A different state is a different face.
    cache.frame(
        &a.clone().with_indicator(Indicator::Ok),
        72,
        72,
        &none,
        &DefaultAppearance::BUILTIN,
    );
    assert_eq!(cache.rendered(), 3);
    cache.frame(&b, 72, 72, &none, &DefaultAppearance::BUILTIN);
    cache.frame(&c, 72, 72, &none, &DefaultAppearance::BUILTIN);
    assert_eq!(cache.len(), 2);
    // `a` was dropped as the oldest.
    cache.frame(&a, 72, 72, &none, &DefaultAppearance::BUILTIN);
    assert_eq!(cache.rendered(), 6);
    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn appearance_sets_colours_size_and_position() {
    let plain = KeyFace::new(FaceStyle::Action, "Go");
    let styled = plain.clone().with_appearance(Some(&KeyAppearance {
        background: Some(Color([200, 0, 0]).into()),
        text_color: Some(Color([0, 255, 0])),
        ..appearance()
    }));
    let frame = render(&styled, 72, 72);
    assert_eq!(frame.pixel(0, 0), Some([200, 0, 0]));
    assert_ne!(rows_with(&frame, [0, 255, 0]), Vec::<u32>::new());
    assert_eq!(rows_with(&frame, WHITE), Vec::<u32>::new());

    // Top, middle and bottom titles.
    let at = |align| {
        let face = plain.clone().with_appearance(Some(&KeyAppearance {
            title_align: Some(align),
            ..appearance()
        }));
        let rows = rows_with(&render(&face, 72, 72), WHITE);
        (rows[0], *rows.last().unwrap())
    };
    let (top, _) = at(TitleAlign::Top);
    let (middle_first, middle_last) = at(TitleAlign::Middle);
    let (_, bottom) = at(TitleAlign::Bottom);
    assert!(top < 12, "{top}");
    assert!(middle_first > 24 && middle_last < 48);
    assert!(bottom > 60, "{bottom}");
    // With an icon, a top title puts the icon below it.
    let top_title = plain
        .clone()
        .with_symbol(Some("cpu"))
        .with_appearance(Some(&KeyAppearance {
            title_align: Some(TitleAlign::Top),
            ..appearance()
        }));
    let rows = rows_with(&render(&top_title, 72, 72), WHITE);
    assert!(rows[0] < 12 && *rows.last().unwrap() > 48, "{rows:?}");

    // Larger fonts use more rows; hidden titles none.
    let height = |size| {
        let face = plain.clone().with_appearance(Some(&KeyAppearance {
            font_size: Some(size),
            ..appearance()
        }));
        rows_with(&render(&face, 72, 72), WHITE).len()
    };
    assert!(height(FontSize::Small) < height(FontSize::Medium));
    assert!(height(FontSize::Large) > height(FontSize::Medium));
    let mut big_key = plain.clone().with_appearance(Some(&KeyAppearance {
        font_size: Some(FontSize::Small),
        ..appearance()
    }));
    let small_on_big = rows_with(&render(&big_key, 120, 120), WHITE).len();
    big_key.appearance = None;
    assert!(rows_with(&render(&big_key, 120, 120), WHITE).len() > small_on_big);
    let hidden = plain.with_appearance(Some(&KeyAppearance {
        show_title: Some(false),
        ..appearance()
    }));
    assert_eq!(
        rows_with(&render(&hidden, 72, 72), WHITE),
        Vec::<u32>::new()
    );
    // Empty keys stay black whatever their appearance says.
    let empty = KeyFace::empty().with_appearance(Some(&KeyAppearance {
        background: Some(Color([1, 2, 3]).into()),
        ..appearance()
    }));
    assert_eq!(render(&empty, 8, 8).pixel(0, 0), Some([0, 0, 0]));
}

#[test]
fn icons_are_drawn_under_titles_and_state_marks() {
    use deckpilot_core::{IconFit, IconRef};
    let icon = icons::import("Halves", &icons::tests::png(&icons::tests::halves(64, 64))).unwrap();
    let mut profile = deckpilot_core::Profile::new("Work");
    let id = icon.id;
    profile.icons.push(icon);
    let icons = Icons::from_profile(&profile);
    let face = KeyFace::new(FaceStyle::Action, "Go")
        .with_symbol(Some("cpu"))
        .with_appearance(Some(&KeyAppearance {
            icon: Some(IconRef {
                id: id.into(),
                fit: IconFit::Fill,
            }),
            ..appearance()
        }))
        .with_indicator(Indicator::Error)
        .with_progress(Some(100));
    let frame = render_with(&face, 72, 72, &icons);
    // The icon fills the key: red on the left, blue on the right.
    assert_eq!(frame.pixel(2, 20), Some([255, 0, 0]));
    assert_eq!(frame.pixel(40, 30), Some([0, 0, 255]));
    // The title sits at the bottom by default, in white on a translucent
    // band (no shadow): the icon is darker behind it, not black.
    let white = rows_with(&frame, WHITE);
    assert!(*white.first().unwrap() > 40);
    let band = frame.pixel(2, *white.first().unwrap()).unwrap();
    assert!(band[0] > 60 && band[0] < 200, "{band:?}");
    assert_eq!(rows_with(&frame, [0, 0, 0]), Vec::<u32>::new());
    // State marks stay on top.
    assert_eq!(frame.pixel(36, 71), Some(BLUE));
    assert!(rows_with(&frame, RED).iter().any(|y| *y < 16));

    // Without the decoded icon (e.g. missing), the key shows its own
    // icon instead, the fallback the app shows too.
    let missing = render(&face, 72, 72);
    assert_eq!(missing.pixel(2, 20), Some(BLACK));
    assert!(rows_with(&missing, WHITE)[0] < 16);
}

#[test]
fn an_ellipsis_is_its_own_character_not_a_question_mark() {
    let draw = |title: &str| render(&KeyFace::new(FaceStyle::Action, title), 72, 72);
    assert_ne!(draw("…").rgb(), draw("?").rgb());
    assert_ne!(draw("Checking…").rgb(), draw("Checking?").rgb());
}

/// The strongest edges of a frame: the mean of the largest tenth of the
/// differences between horizontally neighbouring pixels (luma).
fn edge_contrast(frame: &Frame) -> f32 {
    let luma =
        |p: [u8; 3]| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]);
    let mut steps: Vec<f32> = (0..frame.height())
        .flat_map(|y| (1..frame.width()).map(move |x| (x, y)))
        .filter_map(|(x, y)| Some((luma(frame.pixel(x, y)?) - luma(frame.pixel(x - 1, y)?)).abs()))
        .filter(|step| *step > 8.0)
        .collect();
    steps.sort_by(|a, b| b.total_cmp(a));
    let top = &steps[..(steps.len() / 10).max(1)];
    top.iter().sum::<f32>() / top.len() as f32
}

/// `frame` scaled up `factor` times with bilinear filtering, as a browser
/// shows a small image in a large box.
fn upscaled(frame: &Frame, factor: u32) -> Frame {
    let image =
        image::RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec()).unwrap();
    let scaled = image::imageops::resize(
        &image,
        frame.width() * factor,
        frame.height() * factor,
        image::imageops::FilterType::Triangle,
    );
    let mut out = Frame::filled(scaled.width(), scaled.height(), [0, 0, 0]);
    for (x, y, pixel) in scaled.enumerate_pixels() {
        out.put(x, y, pixel.0);
    }
    out
}

/// The editor asks for a key at the size it shows it (CSS size times the
/// display's pixel ratio): drawn at that size, its icon and title keep
/// sharp edges, where the device's 72 pixels scaled up three times (as the
/// preview did) blur them. The device's own size is drawn as before.
#[test]
fn keys_drawn_at_display_size_are_sharper_than_scaled_up_device_images() {
    let face = KeyFace::new(FaceStyle::Action, "Open Terminal").with_symbol(Some("terminal"));
    let native = render(&face, 72, 72);
    assert_eq!((native.width(), native.height()), (72, 72));
    let display = render(&face, 216, 216);
    assert_eq!((display.width(), display.height()), (216, 216));
    let blurred = upscaled(&native, 3);
    let (sharp, soft) = (edge_contrast(&display), edge_contrast(&blurred));
    assert!(
        sharp > soft * 1.6,
        "drawn at size: {sharp}, scaled up: {soft}"
    );
    // The same layout at both sizes: the ink covers the same share of the
    // key, within a pixel of the small one.
    let bg = BLACK;
    let (l, t, r, b) = ink_box(&native, bg).unwrap();
    let (dl, dt, dr, db) = ink_box(&display, bg).unwrap();
    for (small, large) in [(l, dl), (t, dt), (r, dr), (b, db)] {
        assert!(
            (f64::from(large) / 3.0 - f64::from(small)).abs() <= 1.5,
            "{small} vs {large}"
        );
    }
}

/// Icons stay in proportion: never more than about two fifths of the key
/// above a title, a little over half without one, at every size.
#[test]
fn icons_keep_their_proportions_at_every_size() {
    let titled = KeyFace::new(FaceStyle::Action, "Mute").with_symbol(Some("volume-2"));
    let alone = KeyFace::new(FaceStyle::Action, "").with_symbol(Some("volume-2"));
    let bg = BLACK;
    for side in [72_u32, 96, 120, 144, 216] {
        let key = side as f32;
        let (left, top, right, bottom) = ink_box(&render(&alone, side, side), bg).unwrap();
        let largest = ((right - left + 1).max(bottom - top + 1)) as f32;
        assert!(largest <= key * MARK_ALONE + 1.0, "{side}: {largest}");
        assert!(largest >= key * 0.4, "{side}: {largest}");
        let mark = layout(
            &Request {
                face: &titled,
                look: &appearance(),
                colors: key_colors(&titled, &DefaultAppearance::BUILTIN),
                has_mark: true,
                over_icon: false,
            },
            side,
            side,
        );
        let (_, top, mark_side) = mark.mark.expect("the icon is drawn");
        assert!(
            mark_side <= key * MARK_WITH_TEXT + 0.01,
            "{side}: {mark_side}"
        );
        assert!(mark_side >= key * 0.3, "{side}: {mark_side}");
        // Never on the title.
        assert!(top + mark_side < mark.block.unwrap().0, "{side}");
    }
}

/// A display that is not black, so a transparent key is told apart from
/// a black one.
fn blue_display() -> DefaultAppearance {
    DefaultAppearance {
        key_background: deckpilot_core::KeyBackground::Transparent,
        text_color: Color([255, 224, 0]),
        display_background: Color([0, 0, 90]),
    }
}

#[test]
fn every_kind_of_key_without_colours_gets_the_models_default() {
    for style in [
        FaceStyle::Empty,
        FaceStyle::Action,
        FaceStyle::Workflow,
        FaceStyle::Folder,
        FaceStyle::Back,
        FaceStyle::Notice,
    ] {
        let face = KeyFace::new(style, "");
        // Stream Deck: transparent on the black display, white text.
        let colors = key_colors(&face, &DefaultAppearance::BUILTIN);
        assert_eq!(colors.background, BLACK, "{style:?}");
        assert_eq!(colors.text, [255, 255, 255], "{style:?}");
        assert_eq!(render(&face, 16, 16).pixel(0, 0), Some(BLACK), "{style:?}");
        // Another model: its own display shows through.
        let blue = render_for(&face, 16, 16, &Icons::default(), &blue_display());
        assert_eq!(blue.pixel(0, 0), Some([0, 0, 90]), "{style:?}");
    }
    // The folder's Back key too.
    let back = KeyFace::new(FaceStyle::Back, "")
        .with_symbol(Some(deckpilot_core::device_runtime::symbols::FOLDER_BACK));
    let frame = render_for(&back, 72, 72, &Icons::default(), &blue_display());
    assert_eq!(frame.pixel(0, 0), Some([0, 0, 90]));
    assert!(
        rows_with(&frame, [255, 224, 0]).len() > 10,
        "drawn in the default text colour"
    );
}

#[test]
fn a_keys_own_colours_win_over_its_state_and_the_default() {
    let plain = KeyFace::new(FaceStyle::Action, "Mute");
    let look = |background| KeyFace {
        appearance: Some(KeyAppearance {
            background: Some(background),
            ..KeyAppearance::default()
        }),
        ..plain.clone()
    };
    let defaults = blue_display();
    let black = look(Color([0, 0, 0]).into());
    let white = look(Color([255, 255, 255]).into());
    let transparent = look(deckpilot_core::Background::TRANSPARENT);
    // Explicit black and white are drawn as chosen, not as the default.
    assert_eq!(key_colors(&black, &defaults).background, [0, 0, 0]);
    assert_eq!(key_colors(&white, &defaults).background, [255, 255, 255]);
    // Explicit transparent shows the display, even on a model whose
    // default key background is a colour.
    let opaque = DefaultAppearance {
        key_background: Color([50, 50, 50]).into(),
        ..defaults
    };
    assert_eq!(key_colors(&plain, &opaque).background, [50, 50, 50]);
    assert_eq!(key_colors(&transparent, &opaque).background, [0, 0, 90]);
    // A state's accent fills a key that sets no background, and gives way
    // to one that does (transparent included).
    let mut accented = plain.clone();
    accented.accent = Some(Accent::Green);
    assert_eq!(
        key_colors(&accented, &defaults).background,
        marks::accent_color(Accent::Green)
    );
    let mut chosen = transparent.clone();
    chosen.accent = Some(Accent::Green);
    assert_eq!(key_colors(&chosen, &defaults).background, [0, 0, 90]);
    // Text: the key's own, else the model's.
    let mut inked = plain.clone();
    inked.appearance = Some(KeyAppearance {
        text_color: Some(Color([1, 2, 3])),
        ..KeyAppearance::default()
    });
    assert_eq!(key_colors(&inked, &defaults).text, [1, 2, 3]);
    assert_eq!(key_colors(&inked, &defaults).value, [1, 2, 3]);
    assert_eq!(key_colors(&plain, &defaults).text, [255, 224, 0]);
    // The value line: softer than the title, the accent when highlighted.
    assert_eq!(key_colors(&plain, &DefaultAppearance::BUILTIN).value, MUTED);
    assert_eq!(
        key_colors(&plain.clone().highlighted(true), &defaults).value,
        ACCENT
    );
}

#[test]
fn frames_are_cached_per_model_defaults() {
    let face = KeyFace::new(FaceStyle::Action, "A");
    let mut cache = FrameCache::new(4);
    let icons = Icons::default();
    let builtin = cache.frame(&face, 8, 8, &icons, &DefaultAppearance::BUILTIN);
    let blue = cache.frame(&face, 8, 8, &icons, &blue_display());
    assert_eq!(builtin.pixel(0, 0), Some(BLACK));
    assert_eq!(blue.pixel(0, 0), Some([0, 0, 90]));
    assert_eq!(cache.rendered(), 2);
}

/// A face titled `title` with the icon `icon` and background `background`.
fn with_assets(icon: Option<&str>, background: Option<&str>) -> KeyFace {
    let mut face = KeyFace::new(FaceStyle::Action, "Go");
    face.symbol = Some("cpu".into());
    face.appearance = Some(KeyAppearance {
        icon: icon.map(|id| deckpilot_core::IconRef {
            id: deckpilot_core::AssetRef::parse(id).unwrap(),
            fit: deckpilot_core::IconFit::Fit,
        }),
        background: background.map(|b| deckpilot_core::Background::parse(b).unwrap()),
        ..KeyAppearance::default()
    });
    face
}

#[test]
fn library_icons_are_drawn_from_the_font_in_the_text_colour() {
    let icons = Icons::default();
    let house = render_with(&with_assets(Some("deckpilot.house"), None), 72, 72, &icons);
    let cpu = render_with(&with_assets(None, None), 72, 72, &icons);
    assert_ne!(
        house.rgb(),
        cpu.rgb(),
        "the library icon replaces the action's"
    );
    // Drawn like an action's icon named `house`: the same pixels.
    let mut named = with_assets(None, None);
    named.symbol = Some("house".into());
    assert_eq!(house.rgb(), render_with(&named, 72, 72, &icons).rgb());
    // On the key's background, nothing else filled (transparent around it).
    assert_eq!(house.pixel(0, 0), Some(BLACK));
    // An icon of a later version (not in this library) draws no icon of
    // its own: the key looks as if it set none.
    let later = render_with(
        &with_assets(Some("deckpilot.not-yet-an-icon"), None),
        72,
        72,
        &icons,
    );
    assert_eq!(later.rgb(), cpu.rgb());
}

fn plugin_assets() -> Icons {
    let image = icons::tests::halves(32, 32);
    Icons::default().with_library([
        (
            deckpilot_core::AssetId::parse("acme.tools.logo").unwrap(),
            icons::LibraryAsset::Image(std::sync::Arc::new(image.clone())),
        ),
        (
            deckpilot_core::AssetId::parse("acme.tools.photo").unwrap(),
            icons::LibraryAsset::Image(std::sync::Arc::new(image)),
        ),
        (
            deckpilot_core::AssetId::parse("acme.tools.dusk").unwrap(),
            icons::LibraryAsset::Gradient(deckpilot_core::Gradient {
                colors: vec![Color([0, 200, 0]), Color([0, 0, 200])],
                angle: 180,
            }),
        ),
    ])
}

#[test]
fn background_assets_are_rasterised_over_the_default() {
    let assets = plugin_assets();
    // A built-in gradient: top and bottom differ, no pixel is the default.
    let ocean = render_with(
        &with_assets(None, Some("asset:deckpilot.gradient-ocean")),
        72,
        72,
        &assets,
    );
    assert_ne!(ocean.pixel(0, 0), ocean.pixel(71, 71));
    assert_ne!(ocean.pixel(0, 0), Some(BLACK));
    // A plugin's gradient, top green, bottom blue.
    let dusk = render_with(
        &with_assets(None, Some("asset:acme.tools.dusk")),
        72,
        72,
        &assets,
    );
    assert!(dusk.pixel(36, 0).unwrap()[1] > 180);
    assert!(dusk.pixel(36, 71).unwrap()[2] > 180);
    // A plugin's image covers the key (cropped): red left, blue right.
    let photo = render_with(
        &with_assets(None, Some("asset:acme.tools.photo")),
        72,
        72,
        &assets,
    );
    assert_eq!(photo.pixel(2, 2), Some([255, 0, 0]));
    assert_eq!(photo.pixel(69, 2), Some([0, 0, 255]));
    // The same as the Background Library's tile shows.
    let tile = background_preview(
        Some(&deckpilot_core::Background::parse("asset:acme.tools.dusk").unwrap()),
        72,
        72,
        &assets,
        &DefaultAppearance::BUILTIN,
    );
    assert_eq!(tile.pixel(36, 0), dusk.pixel(36, 0));
}

#[test]
fn unavailable_plugin_assets_fall_back_and_come_back() {
    let face = with_assets(Some("acme.tools.logo"), Some("asset:acme.tools.dusk"));
    let reference = face.appearance.clone();
    // The plugin is disabled or uninstalled: its assets are not here. The
    // key shows the device's default background and no icon of its own.
    let missing = render_with(&face, 72, 72, &Icons::default());
    assert_eq!(
        missing.rgb(),
        render(&with_assets(None, None), 72, 72).rgb()
    );
    assert_eq!(missing.pixel(0, 0), Some(BLACK));
    // The reference is untouched (nothing is rewritten).
    assert_eq!(face.appearance, reference);
    // The plugin is back: the same face draws its assets again.
    let back = render_with(&face, 72, 72, &plugin_assets());
    assert_ne!(back.rgb(), missing.rgb());
    assert_eq!(back.pixel(2, 2), Some([255, 0, 0]), "the icon is drawn");
    let gradient_only = with_assets(None, Some("asset:acme.tools.dusk"));
    let gradient = render_with(&gradient_only, 72, 72, &plugin_assets());
    assert!(
        gradient.pixel(36, 0).unwrap()[1] > 150,
        "the gradient is drawn"
    );
    assert!(plugin_assets().has(&deckpilot_core::AssetRef::parse("acme.tools.logo").unwrap()));
    assert!(!Icons::default().has(&deckpilot_core::AssetRef::parse("acme.tools.logo").unwrap()));
}

#[test]
fn inherit_transparent_black_white_custom_and_assets_stay_distinct() {
    let defaults = DefaultAppearance {
        key_background: Color([50, 50, 50]).into(),
        ..blue_display()
    };
    let fill = |background: Option<&str>| {
        let face = with_assets(None, background);
        render_for(&face, 72, 72, &plugin_assets(), &defaults)
            .pixel(0, 0)
            .unwrap()
    };
    let inherit = fill(None);
    let transparent = fill(Some("transparent"));
    let black = fill(Some("#000000"));
    let white = fill(Some("#ffffff"));
    let yellow = fill(Some("#ffd400"));
    let gradient = fill(Some("asset:deckpilot.gradient-sunset"));
    assert_eq!(inherit, [50, 50, 50]);
    assert_eq!(transparent, [0, 0, 90]);
    assert_eq!(black, [0, 0, 0]);
    assert_eq!(white, [255, 255, 255]);
    assert_eq!(yellow, [255, 212, 0]);
    let all = [inherit, transparent, black, white, yellow, gradient];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
    // An asset background is the key's own choice: it beats the accent.
    let mut accented = with_assets(None, Some("asset:deckpilot.gradient-sunset"));
    accented.accent = Some(Accent::Red);
    assert_eq!(
        count(&render(&accented, 72, 72), marks::accent_color(Accent::Red)),
        0
    );
}
