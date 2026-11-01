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
/// The web client's `--sans`: the menus, the status bar's words, the tiles' labels.
pub const UI_FAMILY: &str = "Nunito";

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
    /// Nunito (bundled), for everything that isn't output.
    pub ui: SharedString,
    /// The `font_name` setting, when it names a font installed here: it comes first, for
    /// every cluster it covers (style.css: `'<name>', var(--mono)`).
    pub user: Option<SharedString>,
    /// `web_font_weight`: the weight of text that isn't bold.
    pub weight: u16,
    /// The weights the user's font comes in (`weights_of`), empty without one.
    pub user_weights: Vec<u16>,
    /// The fonts installed here, to check `font_name` against.
    installed: std::sync::Arc<Vec<String>>,
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
    /// The user's own font (`font_name`).
    User,
}

impl Fonts {
    /// The font the output's metrics (row baseline, `ch`) come from: the user's, if set.
    pub fn base(&self) -> FontClass {
        if self.user.is_some() { FontClass::User } else { FontClass::Primary }
    }

    /// Take the `font_name` and `web_font_weight` settings; whether anything changed.
    pub fn set_user(&mut self, name: &str, weight: u16) -> bool {
        let weight = if weight == 0 { 400 } else { weight.clamp(100, 900) };
        let user: Option<SharedString> = match name.trim() {
            "" => None,
            // The web's own bundled font is the default stack.
            n if n.eq_ignore_ascii_case("JetBrains Mono") => None,
            n if n.eq_ignore_ascii_case("monospace") || n.eq_ignore_ascii_case("ui-monospace") => Some(system_monospace().into()),
            n => self.installed.iter().find(|i| i.eq_ignore_ascii_case(n)).map(|i| i.clone().into()),
        };
        let changed = user != self.user || weight != self.weight;
        self.user_weights = if user.is_some() { weights_of(name) } else { Vec::new() };
        self.user = user;
        self.weight = weight;
        changed
    }

    pub fn font(&self, class: FontClass, bold: bool, italic: bool) -> Font {
        let (family, bundled) = match (class, &self.user) {
            (FontClass::Emoji, _) => return font(self.emoji.clone()),
            (FontClass::Text, _) => (self.fallback.clone(), false),
            (FontClass::User, Some(user)) => (user.clone(), false),
            (FontClass::Primary | FontClass::User, _) => (self.primary.clone(), true),
        };
        let mut f = font(family);
        // `.ansi-bold` is `font-weight: bold`, 700 whatever the base weight.
        let weight = if bold {
            700
        } else if bundled {
            self.primary_weight()
        } else if class == FontClass::User && !self.user_weights.is_empty() {
            // A weight the font has, so the matcher picks the face the picker named.
            crate::font_profile::css_weight_match(self.weight, &self.user_weights)
        } else {
            self.weight
        };
        f.weight = FontWeight(weight as f32);
        if italic {
            f.style = FontStyle::Italic;
        }
        f
    }

    /// The bundled face's weight that draws text of `weight` (one it has).
    fn primary_weight(&self) -> u16 {
        crate::font_profile::css_weight_match(self.weight, &BUNDLED_WEIGHTS)
    }

    /// Which of [`BUNDLED`] draws the primary font, for reading its outlines.
    pub fn bundled_face(&self, bold: bool, italic: bool) -> usize {
        bundled_index(if bold { 700 } else { self.primary_weight() }, italic)
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

/// The weights JetBrains Mono NL comes in. The web's JetBrains Mono is one variable font
/// covering 100-800; GPUI draws no variation axis, so here each weight is a face of its own.
pub const BUNDLED_WEIGHTS: [u16; 8] = [100, 200, 300, 400, 500, 600, 700, 800];

macro_rules! jetbrains {
    ($face:literal) => {
        include_bytes!(concat!("../../../assets/native-gui/fonts/JetBrainsMonoNL-", $face, ".ttf"))
    };
}

/// The bundled JetBrains Mono NL faces, upright then italic for each of
/// [`BUNDLED_WEIGHTS`], indexed by [`bundled_index`].
pub const BUNDLED: [&[u8]; 16] = [
    jetbrains!("Thin"),
    jetbrains!("ThinItalic"),
    jetbrains!("ExtraLight"),
    jetbrains!("ExtraLightItalic"),
    jetbrains!("Light"),
    jetbrains!("LightItalic"),
    jetbrains!("Regular"),
    jetbrains!("Italic"),
    jetbrains!("Medium"),
    jetbrains!("MediumItalic"),
    jetbrains!("SemiBold"),
    jetbrains!("SemiBoldItalic"),
    jetbrains!("Bold"),
    jetbrains!("BoldItalic"),
    jetbrains!("ExtraBold"),
    jetbrains!("ExtraBoldItalic"),
];

/// Nunito Regular and Bold (the web bundles it as a variable font; the weights it uses
/// are 400, 500 and 700).
const UI_FONTS: [&[u8]; 2] = [
    include_bytes!("../../../assets/native-gui/fonts/Nunito-Regular.ttf"),
    include_bytes!("../../../assets/native-gui/fonts/Nunito-Bold.ttf"),
];

/// Which of [`BUNDLED`] draws the primary font at this weight (one of
/// [`BUNDLED_WEIGHTS`]) and style.
pub fn bundled_index(weight: u16, italic: bool) -> usize {
    let step = BUNDLED_WEIGHTS.iter().position(|w| *w == weight).unwrap_or(3);
    2 * step + italic as usize
}

/// Register the bundled fonts and work out the fallback, once at startup.
pub fn load(cx: &mut App) -> Fonts {
    warm_font_db();
    let bundled: Vec<Cow<'static, [u8]>> = BUNDLED.iter().chain(UI_FONTS.iter()).map(|b| Cow::Borrowed(*b)).collect();
    if let Err(e) = cx.text_system().add_fonts(bundled) {
        crate::debug_log(true, &format!("native-gui: bundled fonts failed to load: {e}"));
    }
    let installed = cx.text_system().all_font_names();
    let fallback = WEB_FALLBACKS
        .iter()
        .find(|f| installed.iter().any(|i| i == *f))
        .map(|f| f.to_string())
        .unwrap_or_else(system_monospace);
    Fonts {
        primary: PRIMARY_FAMILY.into(),
        fallback: fallback.into(),
        emoji: emoji_family().into(),
        ui: UI_FAMILY.into(),
        user: None,
        weight: 400,
        user_weights: Vec::new(),
        installed: std::sync::Arc::new(installed),
    }
}

/// The system's fonts, by face: which weights each family comes in. Loaded once, on a
/// thread at start (`warm_font_db`) or at first use; a few hundred name tables, well under
/// a second.
fn font_db() -> &'static fontdb::Database {
    static DB: std::sync::OnceLock<fontdb::Database> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

fn warm_font_db() {
    std::thread::spawn(|| {
        let _ = font_db();
    });
}

/// The weights the `font_name` setting's font comes in here, upright, in order: the
/// bundled JetBrains Mono's for the default, else the faces the system has for the family
/// (a variable font is one face, drawn at its default: GPUI applies no weight axis), else
/// `font_profile::DEFAULT_WEIGHTS` (regular and bold, what most fonts have). The Weight
/// picker steps through these and names them.
pub fn weights_of(name: &str) -> Vec<u16> {
    let family = match name.trim() {
        "" => return BUNDLED_WEIGHTS.to_vec(),
        n if n.eq_ignore_ascii_case("JetBrains Mono") => return BUNDLED_WEIGHTS.to_vec(),
        n if n.eq_ignore_ascii_case("monospace") || n.eq_ignore_ascii_case("ui-monospace") => system_monospace(),
        n => n.to_string(),
    };
    let mut weights: Vec<u16> = font_db()
        .faces()
        .filter(|f| f.style == fontdb::Style::Normal && f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(&family)))
        .map(|f| f.weight.0)
        .collect();
    weights.sort_unstable();
    weights.dedup();
    if weights.is_empty() { crate::font_profile::DEFAULT_WEIGHTS.to_vec() } else { weights }
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
    fn the_weights_a_font_comes_in() {
        assert_eq!(weights_of(""), BUNDLED_WEIGHTS.to_vec());
        assert_eq!(weights_of("jetbrains mono"), BUNDLED_WEIGHTS.to_vec());
        assert_eq!(weights_of("No Such Font Zzz"), crate::font_profile::DEFAULT_WEIGHTS.to_vec());
        // A system font with regular and bold faces, where this machine has it.
        if font_db().faces().any(|f| f.families.iter().any(|(n, _)| n == "DejaVu Sans Mono")) {
            assert_eq!(weights_of("DejaVu Sans Mono"), vec![400, 700]);
        }
    }

    #[test]
    fn each_bundled_face_has_its_weight_and_style() {
        for (k, data) in BUNDLED.iter().enumerate() {
            let face = ttf_parser::Face::parse(data, 0).unwrap();
            let weight = BUNDLED_WEIGHTS[k / 2];
            assert_eq!(face.weight().to_number(), weight, "face {k}");
            assert_eq!(face.is_italic(), k % 2 == 1, "face {k}");
            assert_eq!(bundled_index(weight, k % 2 == 1), k);
            // One family, so a font asked for by name reaches every weight.
            let name = |id| face.names().into_iter().filter(|n| n.name_id == id).find_map(|n| n.to_string());
            let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(ttf_parser::name_id::FAMILY));
            assert_eq!(family.as_deref(), Some(PRIMARY_FAMILY), "face {k}");
        }
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
