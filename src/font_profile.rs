//! Font settings by program and operating system (user ruling 2026-10-17): the native GUI,
//! the WebView GUI, a web browser and the Android app each keep their own fonts on each OS,
//! since a font that looks right on one OS may not on another.
//!
//! A profile is the whole Font tab - the font, its phone/tablet/desktop sizes, weight, line
//! height and spacing - keyed `<program>-<os>` (`PROGRAMS`, `OSES`; the Android app is
//! `app-android`). A program on an OS with no profile of its own uses the shared values
//! (`Settings::font_name` and the `web_font_*` fields), which are also what a client from
//! before the split shows and edits. Changing a font in one gives that program on that OS
//! its own profile, starting from what it showed; the shared values never change from a
//! client that knows about profiles.
//!
//! On the wire: `GlobalSettingsMsg.font_profiles` carries every profile (`None` = a server
//! from before the split, whose client then edits the shared values as before), and
//! `UpdateGlobalSettings.font_profile` one client's own (`None` = unchanged). In
//! settings.dat each field is a `[global]` line `font.<key>.<field>=<value>`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The programs that draw fonts, as the first half of a key: the native GUI, the WebView
/// GUI, a web browser and the Android app.
pub const PROGRAMS: &[&str] = &["native", "webview", "browser", "app"];

/// The operating systems, as the second half of a key.
pub const OSES: &[&str] = &["windows", "macos", "linux", "android", "ios", "chromeos", "other"];

/// One program's fonts on one OS: the Font tab's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontProfile {
    /// The font (`font_name`; empty = the default).
    pub name: String,
    pub size_phone: f32,
    pub size_tablet: f32,
    pub size_desktop: f32,
    pub weight: u16,
    /// The input line's line height multiplier.
    pub line_height: f32,
    /// In px.
    pub letter_spacing: f32,
    /// In px.
    pub word_spacing: f32,
}

pub type FontProfiles = BTreeMap<String, FontProfile>;

/// A client's own profile in `UpdateGlobalSettings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontProfileUpdate {
    pub key: String,
    pub profile: FontProfile,
}

impl FontProfile {
    /// Within the ranges every setter keeps the shared values in; a font name loses any
    /// control character (settings.dat is a line per value).
    pub fn clamped(mut self) -> Self {
        let size = |s: f32| if s.is_finite() { s.clamp(crate::MIN_WEB_FONT_SIZE, 48.0) } else { 14.0 };
        self.name = self.name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
        self.size_phone = size(self.size_phone);
        self.size_tablet = size(self.size_tablet);
        self.size_desktop = size(self.size_desktop);
        self.weight = self.weight.clamp(1, 900);
        let within = |v: f32, lo: f32, hi: f32, dflt: f32| if v.is_finite() { v.clamp(lo, hi) } else { dflt };
        self.line_height = within(self.line_height, 0.5, 3.0, 1.2);
        self.letter_spacing = within(self.letter_spacing, -5.0, 10.0, 0.0);
        self.word_spacing = within(self.word_spacing, -5.0, 20.0, 0.0);
        self
    }

    /// The settings.dat fields, in order, with their values.
    fn fields(&self) -> [(&'static str, String); 8] {
        [
            ("name", self.name.clone()),
            ("size_phone", self.size_phone.to_string()),
            ("size_tablet", self.size_tablet.to_string()),
            ("size_desktop", self.size_desktop.to_string()),
            ("weight", self.weight.to_string()),
            ("line_height", self.line_height.to_string()),
            ("letter_spacing", self.letter_spacing.to_string()),
            ("word_spacing", self.word_spacing.to_string()),
        ]
    }

    /// One settings.dat field; false if it isn't one (or doesn't parse).
    fn set_field(&mut self, field: &str, value: &str) -> bool {
        let num = |v: &str| v.trim().parse::<f32>().ok();
        match field {
            "name" => self.name = value.to_string(),
            "size_phone" => match num(value) { Some(v) => self.size_phone = v, None => return false },
            "size_tablet" => match num(value) { Some(v) => self.size_tablet = v, None => return false },
            "size_desktop" => match num(value) { Some(v) => self.size_desktop = v, None => return false },
            "weight" => match value.trim().parse::<u16>() { Ok(v) => self.weight = v, Err(_) => return false },
            "line_height" => match num(value) { Some(v) => self.line_height = v, None => return false },
            "letter_spacing" => match num(value) { Some(v) => self.letter_spacing = v, None => return false },
            "word_spacing" => match num(value) { Some(v) => self.word_spacing = v, None => return false },
            _ => return false,
        }
        true
    }
}

/// Whether `key` is `<program>-<os>` from `PROGRAMS` and `OSES`.
pub fn valid_key(key: &str) -> bool {
    key.split_once('-').is_some_and(|(program, os)| {
        PROGRAMS.contains(&program) && OSES.contains(&os)
    })
}

/// The OS this Clay was built for, as a key's second half.
pub fn this_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

/// The native GUI's key on this OS.
pub fn native_key() -> &'static str {
    match this_os() {
        "windows" => "native-windows",
        "macos" => "native-macos",
        "android" => "native-android",
        "linux" => "native-linux",
        _ => "native-other",
    }
}

/// The settings.dat lines for every profile (`font.<key>.<field>=<value>`).
pub fn write_lines(profiles: &FontProfiles, out: &mut impl std::io::Write) -> std::io::Result<()> {
    for (key, profile) in profiles {
        for (field, value) in profile.fields() {
            writeln!(out, "font.{key}.{field}={value}")?;
        }
    }
    Ok(())
}

impl crate::Settings {
    /// The shared fonts, as a profile: what a program on an OS without its own uses.
    pub fn shared_font(&self) -> FontProfile {
        FontProfile {
            name: self.font_name.clone(),
            size_phone: self.web_font_size_phone,
            size_tablet: self.web_font_size_tablet,
            size_desktop: self.web_font_size_desktop,
            weight: self.web_font_weight,
            line_height: self.web_font_line_height,
            letter_spacing: self.web_font_letter_spacing,
            word_spacing: self.web_font_word_spacing,
        }
    }

    /// A settings.dat `[global]` line whose key starts with `font.`: one profile field. A
    /// profile's first field starts it from the shared values (read earlier in the file),
    /// so a hand-edited profile missing a field still has one. False if `key` isn't one.
    pub fn read_font_profile_line(&mut self, key: &str, value: &str) -> bool {
        let Some((profile_key, field)) = key.strip_prefix("font.").and_then(|rest| rest.rsplit_once('.')) else {
            return false;
        };
        if !valid_key(profile_key) {
            return false;
        }
        let shared = self.shared_font();
        let profile = self.font_profiles.entry(profile_key.to_string()).or_insert(shared);
        let ok = profile.set_field(field, value);
        *profile = profile.clone().clamped();
        ok
    }

    /// A client's own profile, from `UpdateGlobalSettings`; an unknown key is ignored.
    pub fn set_font_profile(&mut self, update: FontProfileUpdate) {
        if valid_key(&update.key) {
            self.font_profiles.insert(update.key, update.profile.clamped());
        }
    }
}

impl crate::websocket::GlobalSettingsMsg {
    /// The shared fonts the server sent, as a profile.
    pub fn shared_font(&self) -> FontProfile {
        FontProfile {
            name: self.font_name.clone(),
            size_phone: self.web_font_size_phone,
            size_tablet: self.web_font_size_tablet,
            size_desktop: self.web_font_size_desktop,
            weight: self.web_font_weight,
            line_height: self.web_font_line_height,
            letter_spacing: self.web_font_letter_spacing,
            word_spacing: self.web_font_word_spacing,
        }
    }

    /// Write `p` into the shared font fields.
    pub fn put_shared_font(&mut self, p: &FontProfile) {
        self.font_name = p.name.clone();
        self.web_font_size_phone = p.size_phone;
        self.web_font_size_tablet = p.size_tablet;
        self.web_font_size_desktop = p.size_desktop;
        self.web_font_weight = p.weight;
        self.web_font_line_height = p.line_height;
        self.web_font_letter_spacing = p.letter_spacing;
        self.web_font_word_spacing = p.word_spacing;
    }

    /// The fonts `key` (a program on an OS) uses: its own, or the shared ones.
    pub fn font_for(&self, key: &str) -> FontProfile {
        self.font_profiles.as_ref().and_then(|m| m.get(key)).cloned().unwrap_or_else(|| self.shared_font())
    }

    /// Whether `key` has fonts of its own.
    pub fn has_own_font(&self, key: &str) -> bool {
        self.font_profiles.as_ref().is_some_and(|m| m.contains_key(key))
    }

    /// Make `p` the fonts `key` uses: its own profile - or, from a server before the
    /// split (no profiles), the shared fields, the only fonts it keeps.
    pub fn set_font_for(&mut self, key: &str, p: FontProfile) {
        match self.font_profiles.as_mut() {
            Some(m) => {
                m.insert(key.to_string(), p);
            }
            None => self.put_shared_font(&p),
        }
    }

    /// `key`'s own profile, for `UpdateGlobalSettings` (None when it has none).
    pub fn font_update_for(&self, key: &str) -> Option<FontProfileUpdate> {
        let profile = self.font_profiles.as_ref()?.get(key)?.clone();
        Some(FontProfileUpdate { key: key.to_string(), profile })
    }
}

/// The standard weights and their names (CSS's and OpenType's): what every Weight picker
/// shows in place of the number. A font comes in some of these (its faces), or, a variable
/// font in a browser, in every value between two of them.
pub const WEIGHT_NAMES: [(u16, &str); 9] = [
    (100, "Thin"),
    (200, "Extra Light"),
    (300, "Light"),
    (400, "Normal"),
    (500, "Medium"),
    (600, "Semi Bold"),
    (700, "Bold"),
    (800, "Extra Bold"),
    (900, "Black"),
];

/// What a font comes in when nothing says otherwise: regular and bold, as most system
/// fonts do.
pub const DEFAULT_WEIGHTS: [u16; 2] = [400, 700];

/// The name of the standard weight nearest `weight`.
pub fn weight_name(weight: u16) -> &'static str {
    WEIGHT_NAMES.iter().min_by_key(|(w, _)| (*w as i32 - weight as i32).abs()).map_or("Normal", |(_, n)| n)
}

/// The face a browser draws for `wanted` from a family with faces of `weights` (CSS Fonts
/// 4, font matching): 400-500 tries upward to 500 first, lighter wanted tries lighter
/// first, heavier tries heavier first. The native GUI asks for a weight the family has, so
/// every platform's matcher agrees with the browser and with the picker's label.
pub fn css_weight_match(wanted: u16, weights: &[u16]) -> u16 {
    let below = || weights.iter().copied().filter(|w| *w < wanted).max();
    let above = |from: u16| weights.iter().copied().filter(|w| *w >= from).min();
    let found = if weights.contains(&wanted) {
        Some(wanted)
    } else if (400..=500).contains(&wanted) {
        weights.iter().copied().filter(|w| *w > wanted && *w <= 500).min().or_else(below).or_else(|| above(wanted))
    } else if wanted < 400 {
        below().or_else(|| above(wanted))
    } else {
        above(wanted).or_else(below)
    };
    found.unwrap_or(wanted)
}

fn weights_or_default(available: &[u16]) -> &[u16] {
    if available.is_empty() { &DEFAULT_WEIGHTS } else { available }
}

/// The Weight picker's text for `weight` in a font that comes in `available`: the name of
/// the weight it draws (`css_weight_match`), with the number after it only when two of the
/// font's weights share a name (350 and 400, say).
pub fn weight_label(weight: u16, available: &[u16]) -> String {
    let avail = weights_or_default(available);
    let shown = css_weight_match(weight, avail);
    let name = weight_name(shown);
    if avail.iter().filter(|w| weight_name(**w) == name).count() > 1 { format!("{name} ({shown})") } else { name.to_string() }
}

/// The Weight picker's − and +: the previous or next (`up`) weight of `available` after
/// the one `weight` draws; the ends stay put.
pub fn step_weight(weight: u16, available: &[u16], up: bool) -> u16 {
    let avail = weights_or_default(available);
    let i = avail.iter().position(|w| *w == css_weight_match(weight, avail)).unwrap_or(0);
    avail[if up { (i + 1).min(avail.len() - 1) } else { i.saturating_sub(1) }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FontProfile {
        FontProfile {
            name: "Fira Code".into(),
            size_phone: 11.0,
            size_tablet: 13.0,
            size_desktop: 15.5,
            weight: 350,
            line_height: 1.4,
            letter_spacing: 0.5,
            word_spacing: 1.0,
        }
    }

    #[test]
    fn keys_are_a_program_and_an_os() {
        assert!(valid_key("native-windows"));
        assert!(valid_key("browser-ios"));
        assert!(valid_key("app-android"));
        assert!(!valid_key("native"));
        assert!(!valid_key("native-beos"));
        assert!(!valid_key("console-linux"));
        assert!(!valid_key("font.native-linux"));
        assert!(valid_key(native_key()));
    }

    #[test]
    fn a_profile_survives_settings_dat() {
        let mut settings = crate::Settings::default();
        settings.font_profiles.insert("native-macos".into(), sample());
        let mut out = Vec::new();
        write_lines(&settings.font_profiles, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("font.native-macos.name=Fira Code\n"), "{text}");

        let mut back = crate::Settings::default();
        for line in text.lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(back.read_font_profile_line(k, v), "{line}");
        }
        assert_eq!(back.font_profiles, settings.font_profiles);
    }

    #[test]
    fn a_partial_profile_starts_from_the_shared_fonts() {
        let mut settings = crate::Settings {
            font_name: "Consolas".into(),
            web_font_size_desktop: 16.0,
            ..Default::default()
        };
        assert!(settings.read_font_profile_line("font.native-windows.size_desktop", "12"));
        let p = &settings.font_profiles["native-windows"];
        assert_eq!((p.name.as_str(), p.size_desktop), ("Consolas", 12.0));
        // Not a profile line, an unknown key or field, garbage: refused, nothing added.
        assert!(!settings.read_font_profile_line("font_name", "x"));
        assert!(!settings.read_font_profile_line("font.native-beos.name", "x"));
        assert!(!settings.read_font_profile_line("font.native-windows.colour", "x"));
        assert!(!settings.read_font_profile_line("font.native-windows.weight", "heavy"));
        assert_eq!(settings.font_profiles.len(), 1);
    }

    #[test]
    fn a_server_from_before_the_split_sends_no_profiles() {
        let app = crate::App::new();
        let mut v = serde_json::to_value(app.build_global_settings_msg()).unwrap();
        assert!(v.get("font_profiles").is_some_and(|p| p.is_object()));
        v.as_object_mut().unwrap().remove("font_profiles");
        let old: crate::websocket::GlobalSettingsMsg = serde_json::from_value(v).unwrap();
        assert!(old.font_profiles.is_none());
        // Its clients edit the shared fonts, as before the split.
        let mut s = old.clone();
        let mut font = s.font_for("browser-linux");
        font.size_desktop = 11.0;
        s.set_font_for("browser-linux", font);
        assert_eq!(s.web_font_size_desktop, 11.0);
        assert!(s.font_update_for("browser-linux").is_none());
    }

    #[test]
    fn values_are_kept_in_range() {
        let wild = FontProfile {
            name: "Evil\nfont=1".into(),
            size_phone: 1.0,
            size_tablet: f32::NAN,
            size_desktop: 500.0,
            weight: 5000,
            line_height: 9.0,
            letter_spacing: -50.0,
            word_spacing: 50.0,
        }
        .clamped();
        assert_eq!(wild.name, "Evilfont=1");
        assert_eq!(wild.size_phone, crate::MIN_WEB_FONT_SIZE);
        assert_eq!((wild.size_tablet, wild.size_desktop, wild.weight), (14.0, 48.0, 900));
        assert_eq!((wild.line_height, wild.letter_spacing, wild.word_spacing), (3.0, -5.0, 20.0));

        let mut settings = crate::Settings::default();
        settings.set_font_profile(FontProfileUpdate { key: "nope".into(), profile: sample() });
        assert!(settings.font_profiles.is_empty());
        settings.set_font_profile(FontProfileUpdate { key: "webview-linux".into(), profile: sample() });
        assert_eq!(settings.font_profiles["webview-linux"], sample());
    }

    #[test]
    fn a_weight_is_matched_as_a_browser_matches_it() {
        let two = [400, 700];
        // A font with only regular and bold (DejaVu Sans Mono, Consolas, Courier).
        for (wanted, got) in [(1, 400), (101, 400), (451, 400), (500, 400), (501, 700), (900, 700)] {
            assert_eq!(css_weight_match(wanted, &two), got, "{wanted}");
        }
        let eight = [100, 200, 300, 400, 500, 600, 700, 800];
        for (wanted, got) in [(1, 100), (101, 100), (250, 200), (400, 400), (401, 500), (451, 500), (551, 600), (650, 700), (900, 800)] {
            assert_eq!(css_weight_match(wanted, &eight), got, "{wanted}");
        }
    }

    #[test]
    fn weights_are_named_and_stepped_through_what_the_font_has() {
        assert_eq!(weight_name(400), "Normal");
        assert_eq!(weight_name(451), "Medium");
        assert_eq!(weight_name(1), "Thin");
        assert_eq!(weight_name(950), "Black");
        let two = [400, 700];
        assert_eq!(weight_label(451, &two), "Normal");
        assert_eq!(weight_label(600, &two), "Bold");
        assert_eq!(weight_label(400, &[]), "Normal", "no list: regular and bold");
        assert_eq!(step_weight(451, &two, true), 700);
        assert_eq!(step_weight(451, &two, false), 400);
        assert_eq!(step_weight(400, &two, false), 400, "the ends stay put");
        assert_eq!(step_weight(700, &two, true), 700);
        let eight = [100, 200, 300, 400, 500, 600, 700, 800];
        assert_eq!(step_weight(451, &eight, true), 600, "from Medium, the weight 451 draws");
        assert_eq!(step_weight(451, &eight, false), 400);
        assert_eq!(step_weight(850, &eight, true), 800);
        // Two faces with one name are told apart by the number.
        assert_eq!(weight_label(380, &[380, 400, 700]), "Normal (380)");
        assert_eq!(weight_label(400, &[380, 400, 700]), "Normal (400)");
        assert_eq!(weight_label(700, &[380, 400, 700]), "Bold");
        // One face only (a variable font the native GUI draws at its default).
        assert_eq!(step_weight(400, &[400], true), 400);
    }
}
