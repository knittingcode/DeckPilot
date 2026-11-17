//! The key font: 8 x 8 bitmaps, one byte per row, bit 0 the leftmost
//! column.
//!
//! - ASCII and Latin-1 Supplement (`U+0000`–`U+007F`, `U+00A0`–`U+00FF`,
//!   e.g. `ó`, `°`, `±`, `×`, `«`, `©`) come from `font8x8` (MIT), the font
//!   the renderer already used.
//! - Latin Extended-A (`U+0100`–`U+017F`, e.g. `ą ć ę ł ń ś ź ż` and their
//!   capitals) is composed here from a base letter of that font and a
//!   diacritic drawn in the same style: above a lowercase letter in the two
//!   empty top rows; above a capital or a tall lowercase letter in the top
//!   row, the letter compressed by one row below it; ogonek and cedilla in
//!   the bottom row; a stroke drawn through the stem.
//! - A few common symbols: `€`, `•`, and typographic quotes and dashes,
//!   which are drawn as their ASCII forms.
//!
//! Anything else is drawn as `?`, as before. Every glyph is computed from
//! constant data: no font file is read or decoded at run time.

use font8x8::legacy::{BASIC_LEGACY, LATIN_LEGACY};

pub(crate) type Glyph = [u8; 8];

/// `…` in one cell, e.g. for "Checking…".
const ELLIPSIS: Glyph = [0, 0, 0, 0, 0, 0xDB, 0xDB, 0];
const EURO: Glyph = [0x3C, 0x06, 0x3F, 0x06, 0x3F, 0x06, 0x3C, 0x00];
const BULLET: Glyph = [0x00, 0x00, 0x18, 0x3C, 0x3C, 0x18, 0x00, 0x00];

/// A diacritic of Latin Extended-A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Acute,
    Circumflex,
    Tilde,
    Macron,
    Breve,
    Dot,
    Diaeresis,
    Ring,
    DoubleAcute,
    Caron,
    /// The caron of `ď ľ ť Ľ`: a small stroke to the right of the letter.
    Apostrophe,
    Cedilla,
    Ogonek,
    Stroke,
    /// `Ŀ ŀ`: a dot to the right of the letter, at mid height.
    MiddleDot,
    /// `ı`, and the dotless base of `İ`-free marks on `i` and `j`.
    Dotless,
}

/// Latin Extended-A, `U+0100` onwards: the base letter and its mark;
/// `None` for the letters that are not composed (`Ĳ ĳ ĸ Ŋ ŋ Œ œ ſ`).
const EXTENDED_A: [Option<(char, Mark)>; 128] = {
    use Mark::{
        Acute, Apostrophe, Breve, Caron, Cedilla, Circumflex, Diaeresis, Dot, Dotless, DoubleAcute,
        Macron, MiddleDot, Ogonek, Ring, Stroke, Tilde,
    };
    [
        // U+0100
        Some(('A', Macron)),
        Some(('a', Macron)),
        Some(('A', Breve)),
        Some(('a', Breve)),
        Some(('A', Ogonek)),
        Some(('a', Ogonek)),
        Some(('C', Acute)),
        Some(('c', Acute)),
        Some(('C', Circumflex)),
        Some(('c', Circumflex)),
        Some(('C', Dot)),
        Some(('c', Dot)),
        Some(('C', Caron)),
        Some(('c', Caron)),
        Some(('D', Caron)),
        Some(('d', Apostrophe)),
        // U+0110
        Some(('D', Stroke)),
        Some(('d', Stroke)),
        Some(('E', Macron)),
        Some(('e', Macron)),
        Some(('E', Breve)),
        Some(('e', Breve)),
        Some(('E', Dot)),
        Some(('e', Dot)),
        Some(('E', Ogonek)),
        Some(('e', Ogonek)),
        Some(('E', Caron)),
        Some(('e', Caron)),
        Some(('G', Circumflex)),
        Some(('g', Circumflex)),
        Some(('G', Breve)),
        Some(('g', Breve)),
        // U+0120
        Some(('G', Dot)),
        Some(('g', Dot)),
        Some(('G', Cedilla)),
        // `ģ` has its comma above: the descender takes the bottom row.
        Some(('g', Acute)),
        Some(('H', Circumflex)),
        Some(('h', Circumflex)),
        Some(('H', Stroke)),
        Some(('h', Stroke)),
        Some(('I', Tilde)),
        Some(('i', Tilde)),
        Some(('I', Macron)),
        Some(('i', Macron)),
        Some(('I', Breve)),
        Some(('i', Breve)),
        Some(('I', Ogonek)),
        Some(('i', Ogonek)),
        // U+0130
        Some(('I', Dot)),
        Some(('i', Dotless)),
        None,
        None,
        Some(('J', Circumflex)),
        Some(('j', Circumflex)),
        Some(('K', Cedilla)),
        Some(('k', Cedilla)),
        None,
        Some(('L', Acute)),
        Some(('l', Acute)),
        Some(('L', Cedilla)),
        Some(('l', Cedilla)),
        Some(('L', Apostrophe)),
        Some(('l', Apostrophe)),
        Some(('L', MiddleDot)),
        // U+0140
        Some(('l', MiddleDot)),
        Some(('L', Stroke)),
        Some(('l', Stroke)),
        Some(('N', Acute)),
        Some(('n', Acute)),
        Some(('N', Cedilla)),
        Some(('n', Cedilla)),
        Some(('N', Caron)),
        Some(('n', Caron)),
        None,
        None,
        None,
        Some(('O', Macron)),
        Some(('o', Macron)),
        Some(('O', Breve)),
        Some(('o', Breve)),
        // U+0150
        Some(('O', DoubleAcute)),
        Some(('o', DoubleAcute)),
        None,
        None,
        Some(('R', Acute)),
        Some(('r', Acute)),
        Some(('R', Cedilla)),
        Some(('r', Cedilla)),
        Some(('R', Caron)),
        Some(('r', Caron)),
        Some(('S', Acute)),
        Some(('s', Acute)),
        Some(('S', Circumflex)),
        Some(('s', Circumflex)),
        Some(('S', Cedilla)),
        Some(('s', Cedilla)),
        // U+0160
        Some(('S', Caron)),
        Some(('s', Caron)),
        Some(('T', Cedilla)),
        Some(('t', Cedilla)),
        Some(('T', Caron)),
        Some(('t', Apostrophe)),
        Some(('T', Stroke)),
        Some(('t', Stroke)),
        Some(('U', Tilde)),
        Some(('u', Tilde)),
        Some(('U', Macron)),
        Some(('u', Macron)),
        Some(('U', Breve)),
        Some(('u', Breve)),
        Some(('U', Ring)),
        Some(('u', Ring)),
        // U+0170
        Some(('U', DoubleAcute)),
        Some(('u', DoubleAcute)),
        Some(('U', Ogonek)),
        Some(('u', Ogonek)),
        Some(('W', Circumflex)),
        Some(('w', Circumflex)),
        Some(('Y', Circumflex)),
        Some(('y', Circumflex)),
        Some(('Y', Diaeresis)),
        Some(('Z', Acute)),
        Some(('z', Acute)),
        Some(('Z', Dot)),
        Some(('z', Dot)),
        Some(('Z', Caron)),
        Some(('z', Caron)),
        None,
    ]
};

/// The bitmap of `ch`, or `None` when the font has none.
pub(crate) fn lookup(ch: char) -> Option<Glyph> {
    let code = u32::from(ch);
    match code {
        0x00..=0x7F => BASIC_LEGACY.get(code as usize).copied(),
        0xA0..=0xFF => LATIN_LEGACY.get((code - 0xA0) as usize).copied(),
        0x100..=0x17F => {
            EXTENDED_A[(code - 0x100) as usize].and_then(|(base, mark)| compose(base, mark))
        }
        _ => match ch {
            '…' => Some(ELLIPSIS),
            '€' => Some(EURO),
            '•' | '·' => Some(BULLET),
            '‘' | '’' | '‚' | '′' => lookup('\''),
            '“' | '”' | '„' | '″' => lookup('"'),
            '‐' | '‑' | '‒' | '–' | '—' | '−' => lookup('-'),
            _ => None,
        },
    }
}

/// The bitmap of `ch`, `?` when the font has none.
pub(crate) fn glyph(ch: char) -> Glyph {
    lookup(ch).unwrap_or(BASIC_LEGACY[usize::from(b'?')])
}

/// The rightmost lit column of `rows`, if any.
fn rightmost(rows: &[u8]) -> Option<u32> {
    let all = rows.iter().fold(0u8, |acc, row| acc | row);
    (all != 0).then(|| all.ilog2())
}

fn bit(column: u32) -> u8 {
    1u8.checked_shl(column.min(7)).unwrap_or(0)
}

/// `base` with `mark`.
fn compose(base: char, mark: Mark) -> Option<Glyph> {
    let mut rows = lookup(base)?;
    // `i` and `j` lose their dot under a mark above.
    if matches!(base, 'i' | 'j') && !matches!(mark, Mark::Ogonek | Mark::Cedilla) {
        rows[0] = 0;
        rows[1] = 0;
    }
    // Lowercase letters of x-height leave the two top rows empty.
    let short = rows[0] == 0 && rows[1] == 0;
    let right = rightmost(&rows).unwrap_or(6);
    let above = |two: [u8; 2], one: u8, rows: &mut Glyph| {
        if short {
            rows[0] |= two[0];
            rows[1] |= two[1];
        } else {
            // The letter loses its third row (the one its shape repeats
            // most often) and moves down under a one-row mark.
            *rows = [one, 0, rows[0], rows[1], rows[3], rows[4], rows[5], rows[6]];
        }
    };
    match mark {
        Mark::Acute => above([0, 0x38], 0x30, &mut rows),
        Mark::Circumflex => above([0x0C, 0x12], 0x1C, &mut rows),
        Mark::Tilde => above([0x2C, 0x1A], 0x2C, &mut rows),
        Mark::Macron => above([0, 0x1E], 0x3C, &mut rows),
        Mark::Breve => above([0x21, 0x1E], 0x22, &mut rows),
        Mark::Dot => above([0, 0x0C], 0x18, &mut rows),
        Mark::Diaeresis => above([0, 0x36], 0x36, &mut rows),
        Mark::Ring => above([0x0C, 0x0C], 0x1C, &mut rows),
        Mark::DoubleAcute => above([0, 0x36], 0x66, &mut rows),
        Mark::Caron => above([0x12, 0x0C], 0x14, &mut rows),
        Mark::Apostrophe => {
            let column = rightmost(&rows[0..3]).unwrap_or(4) + 2;
            rows[0] |= bit(column);
            rows[1] |= bit(column);
        }
        Mark::Cedilla => rows[7] |= 0x0C,
        Mark::Ogonek => rows[7] |= bit(right.saturating_sub(1)) | bit(right),
        Mark::MiddleDot => {
            let column = rightmost(&rows[0..3]).unwrap_or(4) + 2;
            rows[3] |= bit(column);
        }
        Mark::Stroke => match base {
            'L' => {
                rows[3] |= 0x08;
                rows[4] |= 0x01;
            }
            'l' => {
                rows[3] |= 0x10;
                rows[4] |= 0x02;
            }
            'D' => rows[3] |= 0x01,
            'd' => rows[1] |= 0x48,
            'H' => rows[1] |= 0x7F,
            'h' => rows[1] |= 0x0F,
            'T' => rows[3] |= 0x1E,
            't' => rows[4] |= 0x1E,
            _ => return None,
        },
        Mark::Dotless => {}
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph as text, `#` for a lit dot.
    fn picture(ch: char) -> String {
        glyph(ch)
            .iter()
            .map(|row| {
                (0..8)
                    .map(|c| if row >> c & 1 == 1 { '#' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    const POLISH: &str = "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ";

    #[test]
    fn polish_letters_are_their_own_glyphs() {
        let question = glyph('?');
        for ch in POLISH.chars().chain("°±×÷«»©®µ€•–—“”‘’äöüßéèàçñ".chars())
        {
            let rows = lookup(ch).unwrap_or_else(|| panic!("{ch} has no glyph"));
            assert_ne!(rows, question, "{ch}");
        }
        // Each differs from its base letter and from the others.
        let all: Vec<Glyph> = POLISH.chars().map(glyph).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two Polish letters look alike");
            }
        }
        for (ch, base) in POLISH.chars().zip("acelnoszzACELNOSZZ".chars()) {
            assert_ne!(glyph(ch), glyph(base), "{ch} looks like {base}");
        }
    }

    #[test]
    fn every_letter_of_extended_a_that_is_composed_differs_from_its_base() {
        for (offset, entry) in EXTENDED_A.iter().enumerate() {
            let ch = char::from_u32(0x100 + u32::try_from(offset).unwrap()).unwrap();
            match entry {
                Some((base, _)) => {
                    let rows = lookup(ch).unwrap_or_else(|| panic!("{ch}"));
                    assert_ne!(rows, glyph(*base), "{ch} looks like {base}");
                }
                None => assert_eq!(lookup(ch), None, "{ch}"),
            }
        }
    }

    #[test]
    fn what_the_font_does_not_have_is_a_question_mark() {
        for ch in ['Œ', 'ſ', '中', '😀', '\u{2603}'] {
            assert_eq!(lookup(ch), None, "{ch}");
            assert_eq!(glyph(ch), glyph('?'), "{ch}");
        }
        // ASCII is the font's own.
        assert_eq!(glyph('A'), BASIC_LEGACY[usize::from(b'A')]);
    }

    #[test]
    fn marks_sit_where_they_belong() {
        // ą: the ogonek under the right of the letter, nothing above.
        let a = glyph('ą');
        assert_eq!(a[0] | a[1], 0);
        assert_ne!(a[7], 0);
        // ś: the acute in the empty top rows, the letter unchanged below.
        assert_eq!(glyph('ś')[2..], glyph('s')[2..]);
        assert_ne!(glyph('ś')[1], 0);
        // Ś: a one-row acute, a gap, then the letter.
        let s = glyph('Ś');
        assert_ne!(s[0], 0);
        assert_eq!(s[1], 0);
        // ł: the stroke crosses the stem.
        let l = glyph('ł');
        assert_ne!(l[3] & !glyph('l')[3], 0);
        // Two letters as they are drawn.
        assert_eq!(
            picture('ą'),
            "........\n........\n.####...\n....##..\n.#####..\n##..##..\n.###.##.\n.....##."
        );
        assert_eq!(
            picture('Ś'),
            "....##..\n........\n.####...\n##..##..\n.###....\n...###..\n##..##..\n.####..."
        );
    }
}
