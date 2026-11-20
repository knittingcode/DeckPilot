//! DeckPilot's built-in Icon Library: the Lucide icons of the renderer's
//! icon font, in categories (`assets/library.txt`) with search words
//! (`assets/icon-tags.txt`, from Lucide's tags). A key refers to one as
//! `deckpilot.<name>` ([`deckpilot_core::AssetId`]); it is drawn from the
//! font like an action's icon, in the key's text colour, sharp at every
//! size, on whatever background the key has (nothing else is filled).
//!
//! Everything here is compiled in: the library is there on a clean
//! install, offline, whatever plugins are installed.

use std::sync::LazyLock;

use deckpilot_device::Frame;

use crate::text;

const LIBRARY: &str = include_str!("../assets/library.txt");
const TAGS: &str = include_str!("../assets/icon-tags.txt");

/// One icon of the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryIcon {
    /// Its Lucide name (`house`); the asset id is `deckpilot.<name>`.
    pub name: &'static str,
    /// The categories it is shown in, in library order.
    pub categories: Vec<&'static str>,
    /// Search words (English, from Lucide).
    pub tags: Vec<&'static str>,
}

struct Library {
    categories: Vec<&'static str>,
    icons: Vec<LibraryIcon>,
}

fn lines(text: &'static str) -> impl Iterator<Item = &'static str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

static LIBRARY_DATA: LazyLock<Library> = LazyLock::new(|| {
    let mut categories = Vec::new();
    let mut icons: Vec<LibraryIcon> = Vec::new();
    let mut current = None;
    for line in lines(LIBRARY) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            categories.push(name);
            current = Some(name);
            continue;
        }
        let Some(category) = current else { continue };
        for name in line.split_whitespace() {
            // Only icons the font draws (a test keeps the files in step).
            if !text::has_icon(name) {
                continue;
            }
            match icons.iter_mut().find(|icon| icon.name == name) {
                Some(icon) if !icon.categories.contains(&category) => {
                    icon.categories.push(category);
                }
                Some(_) => {}
                None => icons.push(LibraryIcon {
                    name,
                    categories: vec![category],
                    tags: Vec::new(),
                }),
            }
        }
    }
    for line in lines(TAGS) {
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else { continue };
        if let Some(icon) = icons.iter_mut().find(|icon| icon.name == name) {
            icon.tags = words.collect();
        }
    }
    Library { categories, icons }
});

/// The library's icons, in library order (by category, then as listed).
pub fn library_icons() -> &'static [LibraryIcon] {
    &LIBRARY_DATA.icons
}

/// The library's categories, in the order the Icon Library shows them.
pub fn library_categories() -> &'static [&'static str] {
    &LIBRARY_DATA.categories
}

/// The library's icon `name`, if it has one.
pub fn library_icon(name: &str) -> Option<&'static LibraryIcon> {
    LIBRARY_DATA.icons.iter().find(|icon| icon.name == name)
}

/// The icons matching `query` (every word must be in the name or a search
/// word; case is ignored) in `category` (`None`: all), best first: the name
/// itself, then names starting with the query, then the rest in library
/// order.
pub fn search_icons(query: &str, category: Option<&str>) -> Vec<&'static LibraryIcon> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(u8, usize, &'static LibraryIcon)> = library_icons()
        .iter()
        .enumerate()
        .filter(|(_, icon)| category.is_none_or(|c| icon.categories.contains(&c)))
        .filter_map(|(index, icon)| {
            let matches = |word: &str| {
                icon.name.contains(word) || icon.tags.iter().any(|tag| tag.contains(word))
            };
            if !words.iter().all(|word| matches(word)) {
                return None;
            }
            let rank = if query.is_empty() || icon.name == query.replace(' ', "-") {
                0
            } else if icon.name.starts_with(&query) {
                1
            } else if icon.name.contains(&query) {
                2
            } else {
                3
            };
            Some((rank, index, icon))
        })
        .collect();
    found.sort_by_key(|(rank, index, _)| (*rank, *index));
    found.into_iter().map(|(_, _, icon)| icon).collect()
}

/// The icon `name` as a `side` x `side` PNG: white, its coverage as alpha
/// (transparent elsewhere), drawn from the font exactly as on a key, for
/// the Icon Library's tiles. `None` for a name the font does not have.
pub fn icon_thumbnail_png(name: &str, side: u32) -> Option<Vec<u8>> {
    if !text::has_icon(name) || side == 0 {
        return None;
    }
    let mut frame = Frame::filled(side, side, [0, 0, 0]);
    #[allow(
        clippy::cast_precision_loss,
        reason = "a thumbnail is a few dozen pixels"
    )]
    let extent = side as f32;
    text::draw_icon(&mut frame, name, 0.0, 0.0, extent, [255, 255, 255]);
    let rgba: Vec<u8> = frame
        .rgb()
        .chunks(3)
        .flat_map(|pixel| [255, 255, 255, pixel[0]])
        .collect();
    let image = image::RgbaImage::from_raw(side, side, rgba)?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_is_populated_and_matches_the_font() {
        let icons = library_icons();
        assert!(icons.len() >= 400, "{} icons", icons.len());
        assert!(library_categories().len() >= 12);
        // Every icon of the font is in the library, in some category, and
        // every name the library lists is in the font.
        for name in text::icon_names() {
            let icon = library_icon(name).unwrap_or_else(|| panic!("`{name}` not in library.txt"));
            assert_ne!(icon.categories.len(), 0, "{name}");
        }
        for line in lines(LIBRARY).filter(|line| !line.starts_with('[')) {
            for name in line.split_whitespace() {
                assert!(
                    text::has_icon(name),
                    "library.txt names `{name}`, not in the font"
                );
            }
        }
        for line in lines(TAGS) {
            let name = line.split_whitespace().next().unwrap();
            assert!(text::has_icon(name), "icon-tags.txt names `{name}`");
        }
        // Each category has a sensible number of icons.
        for category in library_categories() {
            let count = icons
                .iter()
                .filter(|i| i.categories.contains(category))
                .count();
            assert!(count >= 10, "{category}: {count}");
        }
    }

    #[test]
    fn icons_are_found_by_name_words_and_category() {
        let names = |found: Vec<&LibraryIcon>| found.iter().map(|i| i.name).collect::<Vec<_>>();
        assert_eq!(search_icons("house", None)[0].name, "house");
        // By a search word: Lucide tags `house` with `home`.
        assert!(names(search_icons("home", None)).contains(&"house"));
        let mic = names(search_icons("mic", None));
        assert_eq!(mic[0], "mic");
        assert!(mic.contains(&"mic-off"));
        assert!(
            search_icons("MIC", None).len() == mic.len(),
            "case is ignored"
        );
        assert_eq!(
            names(search_icons("no such icon at all", None)),
            [] as [&str; 0]
        );
        let media = search_icons("", Some("media"));
        assert!(media.iter().all(|i| i.categories.contains(&"media")));
        assert!(names(media).contains(&"play"));
        assert_eq!(
            names(search_icons("play", Some("weather"))),
            [] as [&str; 0]
        );
        assert_eq!(search_icons("", None).len(), library_icons().len());
    }

    #[test]
    fn thumbnails_are_white_on_transparent() {
        let png = icon_thumbnail_png("house", 32).unwrap();
        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (32, 32));
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "transparent around the icon");
        assert!(
            image
                .pixels()
                .any(|p| p.0[3] == 255 && p.0[..3] == [255, 255, 255])
        );
        assert_eq!(icon_thumbnail_png("no-such-icon", 32), None);
    }
}
