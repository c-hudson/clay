//! The window's images: the web client's icons (index.html's inline SVGs, as files under
//! `assets/native-gui/icons/`), then gpui-component's own (the input's eye, a menu's
//! check mark) for every other path.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("clay/", $name, ".svg"), include_bytes!(concat!("../../../assets/native-gui/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] =
    icons![
        "worlds", "actions", "settings", "find", "tags", "arrow-left", "arrow-right", "menu", "note", "star",
        // The help window's quick-start cards (app.js buildHelpQuickStartCards) and its disclosure chevron.
        "help-connect", "help-switch", "help-send", "help-catchup", "help-actions", "help-yours", "help-remote", "help-notes", "help-chevron",
        // The world editor's section bar.
        "we-server", "we-login", "we-options",
    ];

/// The path of one of our icons, for `gpui::svg().path(..)`.
pub fn icon(name: &str) -> SharedString {
    format!("clay/{name}.svg").into()
}

/// The splash logo (the repo's clay2.png, as the GUI's web assets serve it).
pub const LOGO: &str = "clay/clay2.png";
const LOGO_PNG: &[u8] = include_bytes!("../../../clay2.png");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICONS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(*bytes)));
        }
        if path == LOGO {
            return Ok(Some(Cow::Borrowed(LOGO_PNG)));
        }
        gpui_kit_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| (*p).into()).collect();
        out.extend(gpui_kit_assets::Assets.list(path)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_an_svg_and_others_fall_through() {
        for (path, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.starts_with("<svg") && text.trim_end().ends_with("</svg>"), "{path}");
        }
        assert!(Assets.load(&icon("star")).unwrap().is_some());
        assert!(Assets.load("icons/eye.svg").unwrap().is_some(), "gpui-component's icons are served too");
    }
}
