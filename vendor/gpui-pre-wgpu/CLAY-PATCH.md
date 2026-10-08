# gpui-pre-wgpu 0.3.8, patched for Clay's native GUI

Unmodified copy of crates.io `gpui-pre-wgpu` 0.3.8 (Zed's `gpui_wgpu`, snapshot of
zed@279fe07) except for two fixes in `src/cosmic_text_system.rs`.

## 1. Colour emoji fonts can be requested by name

One condition in `load_family`:

```diff
             if font.as_swash().charmap().map('m') == 0
                 && !allowed_bad_font_names.contains(&postscript_name.as_str())
+                && !check_is_known_emoji_font(&postscript_name)
             {
```

`load_family` drops any face without an "m" glyph, which made it impossible to request
Noto Color Emoji by name. cosmic-text's own fallback reaches Symbola, DejaVu Sans and
Noto Sans Symbols2 before it, so emoji the browser draws in colour (😀, ⚔️, 🟥, keycaps,
ZWJ sequences) came out black and white. With the patch, the output view asks for the
emoji font explicitly for every cluster that presents as emoji, as the WebView does.

## 2. Glyph offsets are kept

`layout_line` built each `ShapedGlyph` position from cosmic-text's `x`/`y` alone and
dropped `x_offset`/`y_offset`, which is where mark attachment (GPOS) puts a combining
accent. So in any text where the font decomposes a precomposed letter, the accent landed
on the following letter: Vietnamese "Tiếng" drew as "Tiếńg". Now:

```diff
-                position: point(glyph.x.into(), glyph.y.into()),
+                position: point(
+                    (glyph.x + glyph.font_size * glyph.x_offset).into(),
+                    (glyph.y - glyph.font_size * glyph.y_offset).into(),
+                ),
```

matching how cosmic-text itself places a glyph (`LayoutGlyph::physical`). This affects
Zed on Linux too, so it is worth sending upstream.

## Wiring

Wired in through `[patch.crates-io]` in the root Cargo.toml. Drop it once upstream GPUI
accepts emoji fonts by name (worth sending upstream).
