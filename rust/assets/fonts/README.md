# Fonts

`Inter-Regular.otf` and `Inter-SemiBold.otf` are Inter 3.13 by The Inter Project Authors
(https://rsms.me/inter/), under the SIL Open Font License 1.1 in `Inter-LICENSE.txt`.

They are subset with fontTools to drop Inter's Private Use Area glyphs (U+E000–U+F8FF),
where the Phosphor icon font keeps its icons, so icons mixed into text resolve to Phosphor:

    pyftsubset Inter-Regular.otf --unicodes="U+0000-DFFF,U+F900-FFFF,U+10000-1FFFF" \
        --layout-features='*' --glyph-names --name-IDs='*' --notdef-outline
