#!/usr/bin/env python3
"""Generate a blank inline-object font; fontTools is needed only to regenerate.

Each BMP private-use glyph reserves N/2048 em and paints no pixels. Native
text shaping therefore wraps images at their exact measured width, while the
semantic text and message source remain separate from this rendering projection.
The font contains no artwork copied from another typeface.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

ROOT = Path(__file__).resolve().parents[1]
MAX_ADVANCE = 8192
font = FontBuilder(2048, isTTF=True)
names = [".notdef"] + [f"spacer{width}" for width in range(1, MAX_ADVANCE + 1)]
font.setupGlyphOrder(names)
cmap = {0xF0000 + width: f"spacer{width}" for width in range(1, MAX_ADVANCE + 1)}
# Some native font loaders select a BMP cmap even when format12 exists. Source
# render sizes need at most32px at12px (5461units), fitting the6400BMP slots.
cmap.update({0xDFFF + width: f"spacer{width}" for width in range(1,6401)})
# Installed GPUI/Cosmic intentionally rejects a primary face without an ASCII
# 'm' glyph (it measures editor columns using that advance). A blank m keeps
# this utility face selectable while all normal text retains Inter.
cmap[ord("m")] = "spacer2048"
font.setupCharacterMap(cmap)
font.setupGlyf({name: TTGlyphPen(None).glyph() for name in names})
font.setupHorizontalMetrics({name: (width, 0) for width, name in enumerate(names)})
# Inter-compatible em metrics avoid altering the text baseline around an object.
font.setupHorizontalHeader(ascent=1984, descent=-494, lineGap=0)
font.setupOS2(fsType=0, fsSelection=0x40, sTypoAscender=1984, sTypoDescender=-494, sTypoLineGap=0,
              usWinAscent=1984, usWinDescent=494)
font.setupNameTable({"familyName": "Stonks Inline Objects", "styleName": "Regular",
                    "uniqueFontIdentifier": "StonksInlineObjects-Regular-1",
                    "fullName": "Stonks Inline Objects Regular",
                    "psName": "StonksInlineObjects-Regular", "version": "Version 1.0",
                    "copyright": "Dedicated to the public domain under CC0-1.0."})
font.setupPost()
font.save(ROOT / "crates/stonks-ui/assets/fonts/Stonks-Inline-Objects.ttf")
