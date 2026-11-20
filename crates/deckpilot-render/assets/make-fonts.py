#!/usr/bin/env python3
"""Builds the key fonts in this directory from their upstream releases.

Usage (fontTools 4.x):

    python3 -I make-fonts.py INTER_DIR LUCIDE_DIR

- INTER_DIR: the unpacked Inter 4.1 release
  (https://github.com/rsms/inter/releases/tag/v4.1), with
  `extras/ttf/Inter-Medium.ttf` and `extras/ttf/Inter-SemiBold.ttf`.
- LUCIDE_DIR: the unpacked npm package `lucide-static` 1.72.0 (the version of
  `lucide-react` the app uses), with `font/lucide.ttf` and
  `font/codepoints.json`.

Inter is subset to the characters keys need (Basic Latin, Latin-1, Latin
Extended-A, common punctuation, arrows, the euro sign), without hinting. Its
pair kerning (GPOS) is copied into a `kern` table, the kerning the
renderer reads (`skrifa`'s `kern` table). Lucide is subset to the icons in
`icons.txt`; the renderer maps an icon name to its code point with
`icons.txt` order, so the list and the font always change together. New
icons are appended (the code points of the others stay). `library.txt` (the
Icon Library's categories) and `icon-tags.txt` (search words from the
package's `tags.json`) name only icons of `icons.txt`; a renderer test checks
all three agree.
"""

import json
import pathlib
import sys

from fontTools import subset
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables._k_e_r_n import KernTable_format_0

HERE = pathlib.Path(__file__).resolve().parent
TEXT = (
    "U+0020-007E,U+00A0-017F,U+2010-2027,U+2030-203A,U+2190-2193,"
    "U+20AC,U+2122,U+2212"
)


def flatten_kerning(font):
    """GPOS pair adjustments (x advance of the first glyph) as glyph pairs."""
    pairs = {}
    gpos = font.get("GPOS")
    if gpos is None:
        return pairs
    table = gpos.table
    kern_lookups = set()
    for record in table.FeatureList.FeatureRecord:
        if record.FeatureTag == "kern":
            kern_lookups.update(record.Feature.LookupListIndex)
    for index in sorted(kern_lookups):
        lookup = table.LookupList.Lookup[index]
        subtables = lookup.SubTable
        if lookup.LookupType == 9:
            subtables = [st.ExtSubTable for st in subtables]
        for st in subtables:
            if getattr(st, "LookupType", 2) != 2:
                continue
            if st.Format == 1:
                for first, pairset in zip(st.Coverage.glyphs, st.PairSet):
                    for record in pairset.PairValueRecord:
                        value = getattr(record.Value1, "XAdvance", 0) or 0
                        if value:
                            pairs.setdefault((first, record.SecondGlyph), value)
            elif st.Format == 2:
                class1 = st.ClassDef1.classDefs
                class2 = st.ClassDef2.classDefs
                seconds = {}
                for glyph in font.getGlyphOrder():
                    seconds.setdefault(class2.get(glyph, 0), []).append(glyph)
                for first in st.Coverage.glyphs:
                    row = st.Class1Record[class1.get(first, 0)]
                    for c2, record in enumerate(row.Class2Record):
                        value = getattr(record.Value1, "XAdvance", 0) or 0
                        if not value:
                            continue
                        for second in seconds.get(c2, []):
                            pairs.setdefault((first, second), value)
    return pairs


def text_font(source, target):
    options = subset.Options()
    options.layout_features = ["kern"]
    options.hinting = False
    options.name_IDs = ["*"]
    options.notdef_outline = True
    font = TTFont(source)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=subset.parse_unicodes(TEXT))
    subsetter.subset(font)
    pairs = flatten_kerning(font)
    kern = newTable("kern")
    kern.version = 0
    kern.kernTables = []
    # A format 0 subtable holds at most 10920 pairs (its length is 16 bits).
    items = sorted(pairs.items())
    for start in range(0, len(items), 8000):
        table = KernTable_format_0()
        table.version = 0
        table.coverage = 1
        table.format = 0
        table.kernTable = dict(items[start : start + 8000])
        kern.kernTables.append(table)
    font["kern"] = kern
    for tag in ("GPOS", "GSUB", "GDEF", "DSIG"):
        if tag in font:
            del font[tag]
    font.save(target)
    print(f"{target.name}: {len(font.getGlyphOrder())} glyphs, {len(pairs)} kerning pairs")


def icon_font(lucide):
    names = [
        line.strip()
        for line in (HERE / "icons.txt").read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    font = TTFont(lucide / "font" / "lucide.ttf")
    # Glyphs are named after the icons, some after an older name of theirs
    # (`house` is `home`); `codepoints.json` maps every current name.
    codepoints = json.loads((lucide / "font" / "codepoints.json").read_text())
    cmap = font.getBestCmap()
    known = set(font.getGlyphOrder())

    def glyph(name):
        return name if name in known else cmap.get(codepoints.get(name, -1))

    missing = [name for name in names if glyph(name) is None]
    if missing:
        sys.exit(f"not in Lucide: {', '.join(missing)}")
    glyphs = [glyph(name) for name in names]
    # Renumber: icon i of icons.txt is U+E000 + i.
    options = subset.Options()
    options.hinting = False
    options.layout_features = []
    options.name_IDs = ["*"]
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(glyphs=glyphs)
    subsetter.subset(font)
    for table in font["cmap"].tables:
        table.cmap = {0xE000 + i: glyph for i, glyph in enumerate(glyphs)}
    font["cmap"].tables = [t for t in font["cmap"].tables if t.isUnicode()]
    target = HERE / "Lucide-Keys.ttf"
    font.save(target)
    print(f"{target.name}: {len(names)} icons")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    inter = pathlib.Path(sys.argv[1])
    lucide = pathlib.Path(sys.argv[2])
    for weight in ("Medium", "SemiBold"):
        text_font(inter / "extras" / "ttf" / f"Inter-{weight}.ttf", HERE / f"Inter-{weight}.ttf")
    icon_font(lucide)


if __name__ == "__main__":
    main()
