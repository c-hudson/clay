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

/// The programs that draw fonts, as the first half of a key, with how they are named.
pub const PROGRAMS: &[(&str, &str)] = &[
    ("native", "the native GUI"),
    ("webview", "the WebView GUI"),
    ("browser", "a web browser"),
    ("app", "the Android app"),
];

/// The operating systems, as the second half of a key.
pub const OSES: &[(&str, &str)] = &[
    ("windows", "Windows"),
    ("macos", "macOS"),
    ("linux", "Linux"),
    ("android", "Android"),
    ("ios", "iOS"),
    ("chromeos", "ChromeOS"),
    ("other", "another OS"),
];

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
        PROGRAMS.iter().any(|(p, _)| *p == program) && OSES.iter().any(|(o, _)| *o == os)
    })
}

/// How a key reads in the Font tab: "the native GUI on Windows", "the Android app".
pub fn label(key: &str) -> String {
    if key == "app-android" {
        return "the Android app".to_string();
    }
    let Some((program, os)) = key.split_once('-') else {
        return key.to_string();
    };
    let program = PROGRAMS.iter().find(|(p, _)| *p == program).map_or(program, |(_, l)| l);
    let os = OSES.iter().find(|(o, _)| *o == os).map_or(os, |(_, l)| l);
    format!("{program} on {os}")
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
        assert_eq!(label("native-windows"), "the native GUI on Windows");
        assert_eq!(label("browser-linux"), "a web browser on Linux");
        assert_eq!(label("app-android"), "the Android app");
        assert!(valid_key(&native_key()));
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
        let mut settings = crate::Settings::default();
        settings.font_name = "Consolas".into();
        settings.web_font_size_desktop = 16.0;
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
}
