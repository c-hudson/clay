//! The output font stack, matching the WebView's.
//!
//! The web client's `--mono` is `'JetBrains Mono', 'Cascadia Code', 'Fira Code',
//! 'Consolas', 'Monaco', 'Lucida Console', monospace`, and its JetBrains Mono is a bundled
//! Latin-only subset (an `@font-face` with a `unicode-range`). So box drawing, CJK, emoji
//! and the rest never come from JetBrains Mono there: they fall through to the next font
//! in the list, then the OS's fallback. Here the full JetBrains Mono NL (no ligatures,
//! like the web's `font-variant-ligatures: none`) is bundled, and each grapheme cluster
//! is given JetBrains Mono only when the subset would have covered it.
//!
//! Emoji follow the browser too: a cluster that presents as emoji (Unicode TR51 - an
//! emoji-default character, U+FE0F, a ZWJ sequence, a flag, a skin tone) is drawn from
//! the colour emoji font even when a text font has a black-and-white glyph for it.

use std::borrow::Cow;

use gpui::{App, Font, FontStyle, FontWeight, SharedString, font};

pub const PRIMARY_FAMILY: &str = "JetBrains Mono NL";

/// The fonts after JetBrains Mono in the web client's list, in order.
const WEB_FALLBACKS: &[&str] = &["Cascadia Code", "Fira Code", "Consolas", "Monaco", "Lucida Console"];

#[derive(Clone, Debug)]
pub struct Fonts {
    pub primary: SharedString,
    /// What the browser uses for a character outside the subset: the first installed font
    /// of the web list, else the system's generic monospace font.
    pub fallback: SharedString,
    /// The system's colour emoji font. (On Linux this needs the vendored GPUI patch in
    /// vendor/gpui-pre-wgpu: stock GPUI refuses to load a font with no "m" by name.)
    pub emoji: SharedString,
}

/// Which font a grapheme cluster is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontClass {
    /// The bundled JetBrains Mono (the web's Latin subset).
    Primary,
    /// The next font in the web's list, or the system monospace font.
    Text,
    /// The colour emoji font.
    Emoji,
}

impl Fonts {
    pub fn font(&self, class: FontClass, bold: bool, italic: bool) -> Font {
        let family = match class {
            FontClass::Primary => self.primary.clone(),
            FontClass::Text => self.fallback.clone(),
            FontClass::Emoji => return font(self.emoji.clone()),
        };
        let mut f = font(family);
        if bold {
            f.weight = FontWeight::BOLD;
        }
        if italic {
            f.style = FontStyle::Italic;
        }
        f
    }
}

/// The font class of one grapheme cluster.
pub fn class_of(cluster: &str) -> FontClass {
    if presents_as_emoji(cluster) {
        FontClass::Emoji
    } else if cluster.chars().all(in_primary_subset) {
        FontClass::Primary
    } else {
        FontClass::Text
    }
}

/// Whether a cluster is drawn as a colour emoji (Unicode TR51 presentation, as browsers
/// apply it with `font-variant-emoji: normal`).
fn presents_as_emoji(cluster: &str) -> bool {
    use unicode_properties::{EmojiStatus, UnicodeEmoji};
    let mut chars = cluster.chars();
    let Some(first) = chars.next() else { return false };
    if cluster.contains('\u{FE0E}') {
        return false;
    }
    if cluster.contains('\u{FE0F}') {
        return true;
    }
    let is_emoji = |c: char| !matches!(c.emoji_status(), EmojiStatus::NonEmoji | EmojiStatus::NonEmojiButEmojiComponent);
    for c in cluster.chars() {
        match c as u32 {
            // Regional indicators (flags), skin-tone modifiers, tag sequences.
            0x1F1E6..=0x1F1FF | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F => return true,
            0x200D if cluster.chars().any(|c| c != '\u{200D}' && is_emoji(c)) => return true,
            _ => {}
        }
    }
    matches!(
        first.emoji_status(),
        EmojiStatus::EmojiPresentation
            | EmojiStatus::EmojiPresentationAndModifierBase
            | EmojiStatus::EmojiPresentationAndEmojiComponent
            | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
    )
}

/// Register the bundled fonts and work out the fallback, once at startup.
pub fn load(cx: &mut App) -> Fonts {
    let bundled: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../../../assets/native-gui/fonts/JetBrainsMonoNL-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/native-gui/fonts/JetBrainsMonoNL-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/native-gui/fonts/JetBrainsMonoNL-Italic.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/native-gui/fonts/JetBrainsMonoNL-BoldItalic.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(bundled) {
        crate::debug_log(true, &format!("native-gui: bundled fonts failed to load: {e}"));
    }
    let installed = cx.text_system().all_font_names();
    let fallback = WEB_FALLBACKS
        .iter()
        .find(|f| installed.iter().any(|i| i == *f))
        .map(|f| f.to_string())
        .unwrap_or_else(system_monospace);
    Fonts { primary: PRIMARY_FAMILY.into(), fallback: fallback.into(), emoji: emoji_family().into() }
}

/// The colour emoji font browsers use on this platform.
fn emoji_family() -> String {
    #[cfg(target_os = "linux")]
    {
        // WebKitGTK asks fontconfig for "emoji".
        if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{family[0]}", "emoji"]).output() {
            let family = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !family.is_empty() {
                return family;
            }
        }
        "Noto Color Emoji".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "Apple Color Emoji".to_string()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "Segoe UI Emoji".to_string()
    }
}

/// The browser's generic `monospace` on this platform.
fn system_monospace() -> String {
    #[cfg(target_os = "linux")]
    {
        // WebKitGTK resolves `monospace` through fontconfig.
        if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{family[0]}", "monospace"]).output() {
            let family = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !family.is_empty() {
                return family;
            }
        }
        "DejaVu Sans Mono".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "Courier".to_string()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "Consolas".to_string()
    }
}

/// Whether the web client's JetBrains Mono subset covers `c` (its `@font-face`
/// `unicode-range` in style.css).
pub fn in_primary_subset(c: char) -> bool {
    matches!(c as u32,
        0x0000..=0x00FF | 0x0131 | 0x0152..=0x0153 | 0x02BB..=0x02BC | 0x02C6 | 0x02DA
        | 0x02DC | 0x0304 | 0x0308 | 0x0329 | 0x2000..=0x206F | 0x20AC | 0x2122 | 0x2191
        | 0x2193 | 0x2212 | 0x2215 | 0xFEFF | 0xFFFD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_presentation_follows_tr51() {
        assert_eq!(class_of("😀"), FontClass::Emoji);
        assert_eq!(class_of("⚔\u{FE0F}"), FontClass::Emoji);
        assert_eq!(class_of("☺\u{FE0E}"), FontClass::Text);
        // Text-default without a selector stays text, as in the browser.
        assert_eq!(class_of("♥"), FontClass::Text);
        assert_eq!(class_of("▶"), FontClass::Text);
        assert_eq!(class_of("🐈\u{200D}⬛"), FontClass::Emoji);
        assert_eq!(class_of("🇺🇸"), FontClass::Emoji);
        assert_eq!(class_of("👋🏽"), FontClass::Emoji);
        assert_eq!(class_of("1\u{FE0F}\u{20E3}"), FontClass::Emoji);
        assert_eq!(class_of("🟥"), FontClass::Emoji);
        assert_eq!(class_of("a"), FontClass::Primary);
        assert_eq!(class_of("─"), FontClass::Text);
    }

    #[test]
    fn subset_covers_latin_but_not_box_drawing_or_emoji() {
        assert!(in_primary_subset('a'));
        assert!(in_primary_subset('é'));
        assert!(in_primary_subset('\u{200d}'));
        assert!(!in_primary_subset('─'));
        assert!(!in_primary_subset('█'));
        assert!(!in_primary_subset('🐈'));
        assert!(!in_primary_subset('漢'));
    }
}
