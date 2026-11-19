use super::*;

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
fn ink_box(frame: &Frame, background: [u8; 3]) -> Option<(u32, u32, u32, u32)> {
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
            assert!(lit(&frame, background(FaceStyle::Action)) > 20);
        }
    }
}

/// The text is a real font: anti-aliased, proportional, and as large on
/// every key relative to its size.
#[test]
fn titles_are_anti_aliased_and_scale_with_the_key() {
    let face = KeyFace::new(FaceStyle::Action, "Mail");
    let bg = background(FaceStyle::Action);
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
    let bg = background(FaceStyle::Action);
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
    assert_eq!(frame.pixel(2, 36), Some(background(FaceStyle::Action)));
}

/// Icon first: the key's icon above the title, the value line under it.
#[test]
fn symbols_are_drawn_above_the_title_and_values_below_it() {
    let bg = background(FaceStyle::Action);
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
            has_mark: true,
            over_icon: false,
        },
        72,
        72,
    );
    let texts: Vec<&str> = placed.lines.iter().map(|line| line.0.as_str()).collect();
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
            has_mark: false,
            over_icon: false,
        },
        72,
        72,
    );
    assert_eq!(placed.lines.last().unwrap().0, "12.4/32 GiB");
    // Without a title, the icon is larger and centred.
    let bare = iconed.clone().with_appearance(Some(&KeyAppearance {
        show_title: Some(false),
        ..appearance()
    }));
    let (left, top, right, bottom) = ink_box(&render(&bare, 72, 72), bg).unwrap();
    // Larger than an icon above a title can be (at most 46 % of the key).
    assert!(
        bottom - top > 36 && right - left > 36,
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
    let bg = background(FaceStyle::Action);
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
    use deckpilot_core::{Accent, Color, Glyph};
    let plain = KeyFace::new(FaceStyle::Action, "Focus");
    let base = render(&plain, 96, 96);

    // An accent is the background; the title stays light on it.
    let mut accented = plain.clone();
    accented.accent = Some(Accent::Green);
    let frame = render(&accented, 96, 96);
    let green = marks::accent_color(Accent::Green);
    assert_eq!(frame.pixel(1, 50), Some(green));
    assert_eq!(count(&frame, WHITE), count(&base, WHITE));
    assert_eq!(count(&frame, background(FaceStyle::Action)), 0);

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
    let line = marks::lighten(background(FaceStyle::Action), 35);
    assert!(count(&frame, line) > 60);
    assert_eq!(count(&frame, WHITE), count(&base, WHITE));

    // The user's background and icon win over accent, glyph and symbol;
    // the user's text colour is the glyph's.
    let mut users = with_glyph.clone();
    users.accent = Some(Accent::Red);
    users.appearance = Some(KeyAppearance {
        background: Some(Color([1, 2, 3])),
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
            id,
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
        cache.frame(&accented, 72, 72, &icons).rgb(),
        cache.frame(&plain, 72, 72, &icons).rgb()
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
        assert!(
            (0..72u32).any(|x| (0..20u32).any(|y| frame.pixel(x, y) == Some(color))),
            "{indicator:?}"
        );
    }
    // Loading is a ring: its centre is not filled.
    let ring = render(&plain.clone().with_indicator(Indicator::Loading), 72, 72);
    let dot = render(&plain.clone().with_indicator(Indicator::Unknown), 72, 72);
    assert!(lit(&ring, background(FaceStyle::Action)) < lit(&dot, background(FaceStyle::Action)));
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
    let first = cache.frame(&a, 72, 72, &none);
    assert_eq!(cache.frame(&a, 72, 72, &none), first);
    assert_eq!(cache.rendered(), 1);
    cache.frame(&a, 96, 96, &none);
    assert_eq!(cache.rendered(), 2);
    // A different state is a different face.
    cache.frame(&a.clone().with_indicator(Indicator::Ok), 72, 72, &none);
    assert_eq!(cache.rendered(), 3);
    cache.frame(&b, 72, 72, &none);
    cache.frame(&c, 72, 72, &none);
    assert_eq!(cache.len(), 2);
    // `a` was dropped as the oldest.
    cache.frame(&a, 72, 72, &none);
    assert_eq!(cache.rendered(), 6);
    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn appearance_sets_colours_size_and_position() {
    use deckpilot_core::Color;
    let plain = KeyFace::new(FaceStyle::Action, "Go");
    let styled = plain.clone().with_appearance(Some(&KeyAppearance {
        background: Some(Color([200, 0, 0])),
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
        background: Some(Color([1, 2, 3])),
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
                id,
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
    assert_eq!(missing.pixel(2, 20), Some(background(FaceStyle::Action)));
    assert!(rows_with(&missing, WHITE)[0] < 16);
}

#[test]
fn an_ellipsis_is_its_own_character_not_a_question_mark() {
    let draw = |title: &str| render(&KeyFace::new(FaceStyle::Action, title), 72, 72);
    assert_ne!(draw("…").rgb(), draw("?").rgb());
    assert_ne!(draw("Checking…").rgb(), draw("Checking?").rgb());
}
