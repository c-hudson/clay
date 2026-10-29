//! Setup (global settings) popup definition
//!
//! Allows editing global application settings.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
    SelectOption,
};

// Field IDs
pub const SETUP_FIELD_MORE_MODE: FieldId = FieldId(1);
pub const SETUP_FIELD_SPELL_CHECK: FieldId = FieldId(2);
pub const SETUP_FIELD_TEMP_CONVERT: FieldId = FieldId(3);
pub const SETUP_FIELD_WORLD_SWITCHING: FieldId = FieldId(4);
pub const SETUP_FIELD_DEBUG: FieldId = FieldId(5);
// Note: show_tags is now a temporary in-memory setting controlled by F2 or /tag command
pub const SETUP_FIELD_INPUT_HEIGHT: FieldId = FieldId(7);
pub const SETUP_FIELD_GUI_THEME: FieldId = FieldId(8);
pub const SETUP_FIELD_TLS_PROXY: FieldId = FieldId(9);
// Note: FieldId(10) was dictionary_path, removed - the spell checker uses the system's dictionary
pub const SETUP_FIELD_EDITOR_SIDE: FieldId = FieldId(11);
// Note: FieldId(12) was mouse_enabled ("Console Mouse"), removed - the console always takes
// the mouse in a popup and a live shell world
pub const SETUP_FIELD_ZWJ: FieldId = FieldId(13);
pub const SETUP_FIELD_ANSI_MUSIC: FieldId = FieldId(14);
/// The new-text marker's style (`Settings::new_indicator`, `NewIndicator::OPTIONS`): a
/// select since 2026-10-28 (it was a toggle; the id is kept).
pub const SETUP_FIELD_NEW_LINE_INDICATOR: FieldId = FieldId(17);
pub const SETUP_FIELD_TTS: FieldId = FieldId(18);
pub const SETUP_FIELD_TTS_SPEAK_MODE: FieldId = FieldId(19);
pub const SETUP_FIELD_SCROLLBACK: FieldId = FieldId(20);
// Note: FieldId(21) was url_shortener, removed - /url now falls back across services automatically
pub const SETUP_FIELD_WRAPSPACE: FieldId = FieldId(22);
pub const SETUP_FIELD_KEYBOARD_VISIBLE: FieldId = FieldId(23);
pub const SETUP_FIELD_TABS: FieldId = FieldId(24);
pub const SETUP_FIELD_ICON_BAR: FieldId = FieldId(25);
// Only visible while SETUP_FIELD_SCROLLBACK ("Archive Input/Output") is checked - see
// update_setup_visibility/update_setup_visibility_def below.
pub const SETUP_FIELD_LOG_INPUT: FieldId = FieldId(26);
/// The Kokoro voice name (src/kokoro.rs; `/tts voices` lists them).
pub const SETUP_FIELD_TTS_VOICE: FieldId = FieldId(27);
/// Import Picture's host (src/picture.rs, `PictureService::OPTIONS`).
pub const SETUP_FIELD_PICTURE_HOST: FieldId = FieldId(28);
/// TF's `%cleardone` (`Settings::clear_input_on_send`).
pub const SETUP_FIELD_CLEAR_INPUT: FieldId = FieldId(29);
/// The console's text-reveal effect (`Settings::text_reveal`, src/reveal.rs).
pub const SETUP_FIELD_TEXT_REVEAL: FieldId = FieldId(30);
/// The console's separator-bar style (`Settings::separator_style`): one global setting,
/// shown in every interface's Settings (user ruling 2026-10-27).
pub const SETUP_FIELD_SEPARATOR_STYLE: FieldId = FieldId(31);

// Button IDs
pub const SETUP_BTN_SAVE: ButtonId = ButtonId(1);
pub const SETUP_BTN_CANCEL: ButtonId = ButtonId(2);
/// Import Settings (`/import`): the master console's only, on the left before the "?",
/// as the GUIs' Settings have it (it left their menu). See [`add_import_button`].
pub const SETUP_BTN_IMPORT: ButtonId = ButtonId(3);

/// World switching mode options
pub fn world_switching_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("unseen_first", "Unseen First"),
        SelectOption::new("alphabetical", "Alphabetical"),
    ]
}

/// Theme options
pub fn theme_options() -> Vec<SelectOption> {
    crate::theme::BUILTIN_THEMES.iter().map(|(name, label)| SelectOption::new(*name, *label)).collect()
}

/// Editor side options
pub fn editor_side_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("left", "Left"),
        SelectOption::new("right", "Right"),
    ]
}

/// Text reveal options (`TextReveal::OPTIONS`): off, the preset speeds, Custom and New
/// Only. When `current` has a typed speed, its entry IS that value (`custom:<n>` labelled
/// "Custom (n)", `new:<n>` labelled "New Only (n)"); otherwise each is the bare entry that
/// asks for a number when Enter is pressed on it, or Save (`text_reveal_needs_number`, the
/// reveal_speed popup). The
/// native GUI's pick is built from this too, so the two lists never differ.
pub fn text_reveal_options(current: &str) -> Vec<SelectOption> {
    let current = crate::TextReveal::from_name(current);
    crate::TextReveal::OPTIONS
        .iter()
        .map(|(name, label)| match current.typed_cps() {
            Some(_) if *name == current.entry() => SelectOption::new(current.name(), current.label()),
            _ => SelectOption::new(*name, *label),
        })
        .collect()
}

/// Which of `text_reveal_options(current)` is `current` (Off when unknown).
pub fn text_reveal_index(current: &str) -> usize {
    let name = crate::TextReveal::from_name(current).name();
    text_reveal_options(current).iter().position(|o| o.value == name).unwrap_or(0)
}

/// The Setup select is on an entry that asks for a number - Custom or New Only, bare or as
/// a value picked again to change it: which one.
pub fn text_reveal_speed_picked(state: &crate::popup::PopupState) -> Option<&'static str> {
    state.get_selected(SETUP_FIELD_TEXT_REVEAL).and_then(crate::TextReveal::speed_entry)
}

/// The Setup select rests on Custom or New Only without its number (the bare entry), which
/// cannot be saved: Save asks for the number first. Cycling never asks - Custom and New Only
/// can be passed, and the field left, on the way to anything else (user rulings 2026-10-26:
/// New Only could not be reached without answering Custom's prompt, and Down must move to
/// the next field); Enter on either entry is what asks.
pub fn text_reveal_needs_number(state: &crate::popup::PopupState) -> bool {
    state.get_selected(SETUP_FIELD_TEXT_REVEAL)
        .is_some_and(|v| crate::TextReveal::speed_entry(v).is_some() && crate::TextReveal::from_name(v).typed_cps().is_none())
}

/// Put `value` (a TextReveal name) on the Setup select: a value with a typed speed takes
/// its entry over, relabelled; a preset selects its own entry. The reveal_speed popup's OK
/// (`custom:<n>`, `new:<n>`) and Cancel (the value before) both come here.
pub fn set_text_reveal_value(state: &mut crate::popup::PopupState, value: &str) {
    let reveal = crate::TextReveal::from_name(value);
    // A bare Custom or New Only (Cancel on a prompt opened from one) stays itself, not Off.
    let name = match crate::TextReveal::speed_entry(value) {
        Some(entry) if reveal.typed_cps().is_none() => entry.to_string(),
        _ => reveal.name(),
    };
    if let Some(FieldKind::Select { options, selected_index }) = state.field_mut(SETUP_FIELD_TEXT_REVEAL).map(|f| &mut f.kind) {
        if reveal.typed_cps().is_some() {
            if let Some(entry) = options.iter_mut().find(|o| crate::TextReveal::speed_entry(&o.value) == Some(reveal.entry())) {
                entry.value = name.clone();
                entry.label = reveal.label();
            }
        }
        if let Some(at) = options.iter().position(|o| o.value == name) {
            *selected_index = at;
        }
    }
}

/// TTS mode options (`TtsMode::OPTIONS`), less Kokoro in a build that can't load it (the
/// static musl console: `kokoro::build_can_load`), which can't speak with it at all.
pub fn tts_mode_options() -> Vec<SelectOption> {
    crate::tts::TtsMode::OPTIONS
        .iter()
        .filter(|(v, _)| *v != "kokoro" || crate::kokoro::build_can_load())
        .map(|(v, l)| SelectOption::new(*v, *l))
        .collect()
}

/// Which of `tts_mode_options()` is `mode`. Kokoro in a build without it is shown as Local,
/// the engine that spoke for it there.
pub fn tts_mode_index(mode: &str) -> usize {
    let options = tts_mode_options();
    let at = |v: &str| options.iter().position(|o| o.value == v);
    at(mode).or_else(|| if mode == "kokoro" { at("local") } else { None }).unwrap_or(0)
}

/// TTS speak mode options (which lines to speak)
pub fn tts_speak_mode_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("all", "All"),
        SelectOption::new("limit", "Limit"),
    ]
}

/// World-tabs ribbon display options (web/GUI/Android only)
pub fn tabs_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("none", "None"),
        SelectOption::new("top", "Show Top Ribbon"),
        SelectOption::new("bottom", "Show Bottom Ribbon"),
    ]
}

/// Icon bar visibility options (web/GUI/Android only)
pub fn icon_bar_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("none", "None"),
        SelectOption::new("app_tablet", "App/Tablet"),
        SelectOption::new("all", "All"),
    ]
}

/// Picture host options (the one list every UI shows, `PictureService::OPTIONS`)
pub fn picture_host_options() -> Vec<SelectOption> {
    crate::picture::PictureService::OPTIONS.iter().map(|(v, l)| SelectOption::new(*v, *l)).collect()
}

/// New-text indicator style options (the one list every UI shows, `NewIndicator::OPTIONS`),
/// the Custom entry shown as `current`'s scheme when it is one, else the first scheme
/// ("Custom (Teal)"): picking it, or Enter on it, opens the scheme picker
/// (glow_scheme.rs). The native GUI's pick is built from this too.
pub fn new_indicator_options(current: &str) -> Vec<SelectOption> {
    let custom = match crate::NewIndicator::from_name(current) {
        c @ crate::NewIndicator::Custom(_) => c,
        _ => crate::NewIndicator::Custom(0),
    };
    crate::NewIndicator::OPTIONS
        .iter()
        .map(|(v, l)| if *v == crate::NewIndicator::CUSTOM { SelectOption::new(custom.name(), custom.label()) } else { SelectOption::new(*v, *l) })
        .collect()
}

/// Which of `new_indicator_options(current)` is `current` (None when unknown).
pub fn new_indicator_index(current: &str) -> usize {
    let name = crate::NewIndicator::from_name(current).name();
    new_indicator_options(current).iter().position(|o| o.value == name).unwrap_or(0)
}

/// The Setup select rests on the Custom entry.
pub fn new_indicator_custom_picked(state: &crate::popup::PopupState) -> bool {
    state.get_selected(SETUP_FIELD_NEW_LINE_INDICATOR).is_some_and(crate::NewIndicator::is_custom_name)
}

/// Put `value` (a NewIndicator name) on the Setup select: a scheme takes the Custom entry
/// over, relabelled; any other value selects its own entry.
pub fn set_new_indicator_value(state: &mut crate::popup::PopupState, value: &str) {
    let indicator = crate::NewIndicator::from_name(value);
    let name = indicator.name();
    if let Some(FieldKind::Select { options, selected_index }) = state.field_mut(SETUP_FIELD_NEW_LINE_INDICATOR).map(|f| &mut f.kind) {
        if matches!(indicator, crate::NewIndicator::Custom(_)) {
            if let Some(entry) = options.iter_mut().find(|o| crate::NewIndicator::is_custom_name(&o.value)) {
                entry.value = name.clone();
                entry.label = indicator.label();
            }
        }
        if let Some(at) = options.iter().position(|o| o.value == name) {
            *selected_index = at;
        }
    }
}

/// Separator-bar style options (the one list every UI shows, `SeparatorStyle::OPTIONS`)
pub fn separator_style_options() -> Vec<SelectOption> {
    crate::SeparatorStyle::OPTIONS.iter().map(|(v, l)| SelectOption::new(*v, *l)).collect()
}

/// Create the setup popup definition with current values
#[allow(clippy::too_many_arguments)]
pub fn create_setup_popup(
    more_mode: bool,
    spell_check: bool,
    temp_convert: bool,
    world_switching: &str,
    debug: bool,
    input_height: i64,
    gui_theme: &str,
    tls_proxy: bool,
    editor_side: &str,
    zwj_enabled: bool,
    ansi_music: bool,
    new_indicator: &str,
    tts_mode: &str,
    tts_speak_mode: &str,
    tts_voice: &str,
    scrollback: bool,
    log_input: bool,
    wrapspace: i64,
    keyboard_always_visible: bool,
    tabs: &str,
    icon_bar: &str,
    picture_host: &str,
    clear_input_on_send: bool,
    text_reveal: &str,
    separator_style: &str,
) -> PopupDefinition {
    let world_switching_idx = if world_switching == "alphabetical" { 1 } else { 0 };
    let gui_theme_idx = crate::theme::BUILTIN_THEMES.iter().position(|(name, _)| *name == gui_theme).unwrap_or(0);
    let editor_side_idx = if editor_side == "right" { 1 } else { 0 };
    let text_reveal_idx = text_reveal_index(text_reveal);
    let tts_mode_idx = tts_mode_index(tts_mode);
    let tts_speak_mode_idx = if tts_speak_mode == "limit" { 1 } else { 0 };
    let tabs_idx = tabs_options().iter().position(|o| o.value == tabs).unwrap_or(0);
    let icon_bar_idx = icon_bar_options().iter().position(|o| o.value == icon_bar).unwrap_or(1);
    let picture_host_idx = picture_host_options().iter().position(|o| o.value == picture_host).unwrap_or(0);
    let separator_style_idx = separator_style_options().iter().position(|o| o.value == separator_style).unwrap_or(0);
    let new_indicator_idx = new_indicator_index(new_indicator);

    let mut def = PopupDefinition::new(PopupId("setup"), "Setup")
        .with_field(Field::new(
            SETUP_FIELD_MORE_MODE,
            "More Mode",
            FieldKind::toggle(more_mode),
        ))
        .with_field(Field::new(
            SETUP_FIELD_SPELL_CHECK,
            "Spell Check",
            FieldKind::toggle(spell_check),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TEMP_CONVERT,
            "Temp Convert",
            FieldKind::toggle(temp_convert),
        ))
        .with_field(Field::new(
            SETUP_FIELD_WORLD_SWITCHING,
            "World Switching",
            FieldKind::select(world_switching_options(), world_switching_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_DEBUG,
            "Debug",
            FieldKind::toggle(debug),
        ))
        .with_field(Field::new(
            SETUP_FIELD_INPUT_HEIGHT,
            "Input Height",
            FieldKind::number(input_height),
        ))
        .with_field(Field::new(
            SETUP_FIELD_CLEAR_INPUT,
            "Clear Input on Send",
            FieldKind::toggle(clear_input_on_send),
        ))
        .with_field(Field::new(
            SETUP_FIELD_GUI_THEME,
            "GUI Theme",
            FieldKind::select(theme_options(), gui_theme_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TLS_PROXY,
            "TLS Proxy",
            FieldKind::toggle(tls_proxy),
        ))
        .with_field(Field::new(
            SETUP_FIELD_EDITOR_SIDE,
            "Editor Side",
            FieldKind::select(editor_side_options(), editor_side_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_ZWJ,
            "ZWJ Sequence",
            FieldKind::toggle(zwj_enabled),
        ))
        .with_field(Field::new(
            SETUP_FIELD_ANSI_MUSIC,
            "ANSI Music",
            FieldKind::toggle(ansi_music),
        ))
        .with_field(Field::new(
            SETUP_FIELD_NEW_LINE_INDICATOR,
            "New Indicator",
            FieldKind::select(new_indicator_options(new_indicator), new_indicator_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TEXT_REVEAL,
            "Text Reveal",
            FieldKind::select(text_reveal_options(text_reveal), text_reveal_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_KEYBOARD_VISIBLE,
            "Keyboard Visible",
            FieldKind::toggle(keyboard_always_visible),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TTS,
            "TTS",
            FieldKind::select(tts_mode_options(), tts_mode_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TTS_SPEAK_MODE,
            "Speak Mode",
            FieldKind::select(tts_speak_mode_options(), tts_speak_mode_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TTS_VOICE,
            "Kokoro Voice",
            FieldKind::text_with_placeholder(tts_voice, crate::kokoro::DEFAULT_VOICE),
        ))
        .with_field(Field::new(
            SETUP_FIELD_SCROLLBACK,
            "Archive Input/Output",
            FieldKind::toggle(scrollback),
        ))
        .with_field(Field::new(
            SETUP_FIELD_LOG_INPUT,
            "Log Input",
            FieldKind::toggle(log_input),
        ))
        .with_field(Field::new(
            SETUP_FIELD_WRAPSPACE,
            "Wrap Space",
            FieldKind::number_range(wrapspace, 0, 20),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TABS,
            "Tabs",
            FieldKind::select(tabs_options(), tabs_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_ICON_BAR,
            "Icon Bar",
            FieldKind::select(icon_bar_options(), icon_bar_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_PICTURE_HOST,
            "Picture Host",
            FieldKind::select(picture_host_options(), picture_host_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_SEPARATOR_STYLE,
            "Separator Style",
            FieldKind::select(separator_style_options(), separator_style_idx),
        ))
        .with_button(Button::new(SETUP_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_button(Button::new(SETUP_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_layout(PopupLayout {
            label_width: 19,
            min_width: 40,
            // Wide enough on an 80-column terminal for the longest Picture Host
            // choice; the popup is only as wide as its widest field needs.
            max_width_percent: 80,
            center_horizontal: true,
            // Top-aligned (matches world_selector.rs) so the popup's height budget reserves
            // the separator+input rows at the bottom instead of centering blind to them —
            // this popup has 19 fields and can otherwise size/position itself to overlap
            // the input pane on a short terminal.
            center_vertical: false,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(setup_help_text());

    update_setup_visibility_def(&mut def);

    def
}

/// The Import button (Import Settings), for the master console (a remote console has no
/// `/import`, as its `/menu` has no Import Settings).
pub fn add_import_button(def: &mut PopupDefinition) {
    // Left-aligned buttons draw in the order they're defined: first is before the "?".
    def.buttons.insert(0, Button::new(SETUP_BTN_IMPORT, "Import").left_align().with_shortcut('I'));
    // Room for both groups with a gap between: at 40 "[ ? ]" touched "[Cancel]".
    def.layout.min_width = def.layout.min_width.max(44);
}

/// Update visibility of the Log Input field based on the current Archive Input/Output
/// (SETUP_FIELD_SCROLLBACK) toggle state - it's only shown once that's checked (see
/// SETUP_FIELD_LOG_INPUT's doc comment). Mirrors update_web_visibility in web.rs.
pub fn update_setup_visibility(state: &mut crate::popup::PopupState) {
    let show_log_input = state.get_bool(SETUP_FIELD_SCROLLBACK).unwrap_or(false);
    if let Some(field) = state.field_mut(SETUP_FIELD_LOG_INPUT) {
        field.visible = show_log_input;
    }
}

/// Same as `update_setup_visibility` but operates directly on a `PopupDefinition` (used
/// at creation time, before a `PopupState` wraps it). Mirrors update_web_visibility_def.
fn update_setup_visibility_def(def: &mut PopupDefinition) {
    let show_log_input = def.get_field(SETUP_FIELD_SCROLLBACK)
        .and_then(|f| if let FieldKind::Toggle { value } = &f.kind { Some(*value) } else { None })
        .unwrap_or(false);
    if let Some(field) = def.get_field_mut(SETUP_FIELD_LOG_INPUT) {
        field.visible = show_log_input;
    }
    // Only what acts in a console: these are the web page's and the GUI's. Hidden rather
    // than left out, so Save sends their values back unchanged.
    for id in CONSOLE_HIDDEN_FIELDS {
        if let Some(field) = def.get_field_mut(id) {
            field.visible = false;
        }
    }
    // Kokoro's voice, where this build can't speak with Kokoro (user request 2026-10-28).
    if !crate::kokoro::build_can_load() {
        if let Some(field) = def.get_field_mut(SETUP_FIELD_TTS_VOICE) {
            field.visible = false;
        }
    }
}

/// Settings the console's `/setup` (and the SSH console's) doesn't show: they have no
/// effect on a console.
pub const CONSOLE_HIDDEN_FIELDS: [FieldId; 4] =
    [SETUP_FIELD_GUI_THEME, SETUP_FIELD_KEYBOARD_VISIBLE, SETUP_FIELD_TABS, SETUP_FIELD_ICON_BAR];

/// Help text for the Setup popup
fn setup_help_text() -> Vec<String> {
    vec![
        "Setup - Global Settings",
        "",
        "Import (button): copy settings, theme and",
        "  keybindings from another Clay (/import). Setup",
        "  closes first; the import replaces what it shows.",
        "",
        "More Mode: Pauses output when a full screen of text",
        "  arrives. Press Tab to release one page, Esc+J to",
        "  release all. Keeps you from missing important text.",
        "",
        "Spell Check: Underlines misspelled words as you type.",
        "  Press Ctrl+Q for spelling suggestions.",
        "",
        "Temp Convert: Automatically converts Fahrenheit temps",
        "  in MUD output to Celsius (shown in parentheses).",
        "",
        "World Switching: Controls Up/Down world switch order.",
        "  'Unseen First' prioritizes worlds with new activity.",
        "  'Alphabetical' cycles worlds in name order.",
        "",
        "Debug: Enables debug logging to ~/.clay/debug.log.",
        "",
        "Input Height: Number of input lines visible (1-10).",
        "",
        "Clear Input on Send: Off (TinyFugue's default), each",
        "  line you send stays in the input area above the",
        "  next one until it scrolls off the top; Ctrl+L",
        "  clears them. On, Enter clears the input area.",
        "  Phones always clear. TF's %cleardone.",
        "",
        "TLS Proxy: Keeps a background proxy process alive",
        "  during hot reload so TLS connections survive.",
        "",
        "Editor Side: Which side the split-screen editor opens.",
        "",
        "ZWJ Sequence: For terminals that support combined",
        "  emoji (ZWJ). If unsupported, enabling this shows",
        "  two separate emoji instead of one combined one.",
        "",
        "ANSI Music: Play ANSI music sequences from MUDs.",
        "",
        "New Indicator: How text that arrived while you were",
        "  in another world is marked. Character: a green ▶",
        "  in front of each line. Background: a green glow",
        "  behind its first 4 characters, fading to the",
        "  theme's background. Custom: the same glow in",
        "  another colour; press Enter on it to pick one of",
        "  ten. None: not marked. Leaving the world or Ctrl+L",
        "  clears the marks, in any style.",
        "",
        "Text Reveal: Type newly arrived lines, and the screen a",
        "  world switch shows, onto the screen behind a glitching",
        "  cursor (Slow 30, Normal 60, Fast 120, Super Fast 240",
        "  characters a second, or Custom: press Enter on it to",
        "  type the number; a wrapped line's rows together). New",
        "  Only asks for a number the same way, but types on only",
        "  text not shown yet: a switch draws what you saw before",
        "  whole.",
        "  Only how the console draws them: scrolling and paging",
        "  are unchanged. Each interface keeps its own.",
        "",
        "TTS: This console's text-to-speech for MUD output.",
        "  Each interface keeps its own, set in its Settings.",
        "  Off: disabled. Local: uses espeak/say subprocess.",
        "  Kokoro: the Kokoro neural voice, offline; needs",
        "  /tts install once (230 MB in ~/.clay/tts); not in",
        "  the static Linux console build.",
        "  Edge: Microsoft Edge neural TTS (needs internet).",
        "  The web page and the Android app speak with the",
        "  device's own voice.",
        "  Use /say <text> to speak manually when TTS is off.",
        "",
        "Kokoro Voice: which of Kokoro's 54 voices speaks,",
        "  e.g. af_heart, am_michael, bf_emma; /tts voices",
        "  lists them all.",
        "",
        "Archive Input/Output: Saves all world output to",
        "  ~/.clay/scrollback.db for permanent storage.",
        "  Enables /recall -D <pattern> to search the archive",
        "  and pg-up past the top of the scrollback buffer.",
        "  Changes take effect on next restart or /reload.",
        "",
        "Log Input: Only shown while Archive Input/Output is",
        "  on. Also writes your typed commands to the",
        "  per-world log file (if Log is on for that world).",
        "  Off by default. Does not affect /recall -i, which",
        "  always works regardless of this setting.",
        "",
        "Wrap Space: Number of spaces to hang-indent wrapped",
        "  continuation lines of long MUD output (0 = off).",
        "  Like TinyFugue's wrapspace, but defaults to 0.",
        "",
        "Picture Host: Where Import Picture uploads a picture",
        "  (dragged onto the terminal or window, or chosen from",
        "  the menu in the web/GUI/Android UI); its link goes",
        "  into the input line. '+ URL Short' runs the link",
        "  through /url's shortening. '(2d)': tmpfiles.org",
        "  deletes it after 2 days. Uploads are public. 'Off'",
        "  turns picture uploads off.",
        "",
        "Separator Style: How the console draws the bar above",
        "  the input. Clay: gaps as spaces on the status bar",
        "  colour, the look of the GUIs and the web page.",
        "  TinyFugue: gaps filled with underscores on the",
        "  terminal background. One setting for every",
        "  interface; only a console's bar changes.",
    ].into_iter().map(|s| s.to_string()).collect()
}

/// A Setup popup at the defaults but for its Text Reveal, for the prompt's tests.
#[cfg(test)]
pub fn create_setup_popup_for_test(text_reveal: &str) -> PopupDefinition {
    create_setup_popup(
        true, true, false, "unseen_first",
        false, 3, "dark", false, "left", false, true,
        "none", "off", "words", "af_heart", false, false, 0, true, "none", "app_tablet", "catbox", false,
        text_reveal, "web",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    /// The Text Reveal select: Custom and New Only are the last entries, bare until a
    /// number is chosen, then the value itself, relabelled; Cancel's restore selects a
    /// preset again.
    #[test]
    fn the_text_reveal_select_shows_a_custom_value_on_its_custom_entry() {
        let names: Vec<String> = text_reveal_options("fast").iter().map(|o| o.value.clone()).collect();
        assert_eq!(names, ["off", "slow", "normal", "fast", "superfast", "custom", "new"]);
        assert_eq!(text_reveal_index("fast"), 3);
        assert_eq!(text_reveal_index("nonsense"), 0);
        let custom = text_reveal_options("custom:150");
        assert_eq!((custom[5].value.as_str(), custom[5].label.as_str()), ("custom:150", "Custom (150)"));
        assert_eq!((custom[6].value.as_str(), custom[6].label.as_str()), ("new", "New Only"));
        assert_eq!(text_reveal_index("custom:150"), 5);
        assert_eq!(text_reveal_index("custom:0"), 0, "out of range is Off");
        let new_only = text_reveal_options("new:90");
        assert_eq!((new_only[5].value.as_str(), new_only[5].label.as_str()), ("custom", "Custom"));
        assert_eq!((new_only[6].value.as_str(), new_only[6].label.as_str()), ("new:90", "New Only (90)"));
        assert_eq!(text_reveal_index("new:90"), 6);

        let mut state = PopupState::new(create_setup_popup_for_test("normal"));
        assert_eq!(text_reveal_speed_picked(&state), None);
        set_text_reveal_value(&mut state, "new:40");
        assert_eq!(state.get_selected(SETUP_FIELD_TEXT_REVEAL), Some("new:40"));
        assert_eq!(text_reveal_speed_picked(&state), Some("new"));
        set_text_reveal_value(&mut state, "custom:75");
        assert_eq!(state.get_selected(SETUP_FIELD_TEXT_REVEAL), Some("custom:75"));
        assert_eq!(text_reveal_speed_picked(&state), Some("custom"));
        let label = match &state.field(SETUP_FIELD_TEXT_REVEAL).unwrap().kind {
            FieldKind::Select { options, selected_index } => options[*selected_index].label.clone(),
            _ => unreachable!(),
        };
        assert_eq!(label, "Custom (75)");
        set_text_reveal_value(&mut state, "slow");
        assert_eq!(state.get_selected(SETUP_FIELD_TEXT_REVEAL), Some("slow"));
        // Cycling onto the Custom entry again offers the kept number.
        set_text_reveal_value(&mut state, "custom:75");
        assert_eq!(state.get_selected(SETUP_FIELD_TEXT_REVEAL), Some("custom:75"));
    }

    #[test]
    fn test_setup_popup_creation() {
        let def = create_setup_popup(
            true, true, false, "unseen_first",
            false, 3, "dark", false, "left", false, true,
            "none", "off", "words", "af_heart", false, false, 0, true, "none", "app_tablet", "catbox", false,
            "off", "web",
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("setup"));
        assert_eq!(state.definition.title, "Setup");
        assert_eq!(state.definition.fields.len(), 25);
        assert_eq!(state.definition.buttons.len(), 3); // ?, Cancel, Save
    }

    #[test]
    fn the_master_consoles_import_button_comes_before_the_help_button() {
        let mut def = create_setup_popup(
            true, true, false, "unseen_first",
            false, 3, "dark", false, "left", false, true,
            "none", "off", "words", "af_heart", false, false, 0, true, "none", "app_tablet", "catbox", false,
            "off", "web",
        );
        add_import_button(&mut def);
        let left: Vec<&str> = def.buttons.iter().filter(|b| b.left_align).map(|b| b.label.as_str()).collect();
        assert_eq!(left, ["Import", "?"], "left-aligned buttons draw in this order");
        let right: Vec<&str> = def.buttons.iter().filter(|b| !b.left_align).map(|b| b.label.as_str()).collect();
        assert_eq!(right, ["Cancel", "Save"]);
        assert!(def.layout.min_width >= 44, "room for both groups with a gap");
    }

    /// Every Picture Host choice is drawn whole on an 80-column terminal, tall enough to
    /// show every field or so short the fields scroll (a scroll bar takes a column).
    #[test]
    fn every_picture_host_choice_fits_an_80_column_console() {
        use ratatui::{backend::TestBackend, Terminal};
        for height in [45, 20] {
            for (value, label) in crate::picture::PictureService::OPTIONS {
                let mut def = create_setup_popup(
                    true, true, false, "unseen_first",
                    false, 3, "dark", false, "left", false, true,
                    "none", "off", "words", "af_heart", false, false, 0, true, "none", "app_tablet", value, false,
                    "off", "web",
                );
                add_import_button(&mut def);
                let mut state = PopupState::new(def);
                state.visible = true;
                state.select_field(SETUP_FIELD_PICTURE_HOST);
                let mut terminal = Terminal::new(TestBackend::new(80, height)).unwrap();
                terminal
                    .draw(|f| crate::popup::console_renderer::render_popup(f, &mut state, &crate::encoding::Theme::Dark))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let rows: Vec<String> = (0..height)
                    .map(|y| (0..80).map(|x| buffer.get(x, y).symbol().to_string()).collect())
                    .collect();
                let row = rows.iter().find(|r| r.contains("Picture Host:")).expect("the Picture Host row");
                assert!(row.contains(&format!("[ {} ]", label)), "{label:?} cut off at height {height}: {row:?}");
            }
        }
    }

    /// The web page's and the GUI's settings are hidden in a console, but still hold the
    /// values Save sends back.
    #[test]
    fn a_console_hides_what_only_the_gui_and_web_use() {
        let def = create_setup_popup(
            true, false, true, "alphabetical",
            true, 5, "light", true, "left", true, true,
            "background", "edge", "sentences", "bm_george", true, false, 4, false, "top", "all", "sxcu+short", true,
            "fast", "tinyfugue",
        );
        for id in CONSOLE_HIDDEN_FIELDS {
            assert!(!def.get_field(id).unwrap().visible, "{id:?} shown in the console");
        }
        let state = PopupState::new(def);
        assert_eq!(state.get_selected(SETUP_FIELD_GUI_THEME), Some("light"));
        assert_eq!(state.get_bool(SETUP_FIELD_KEYBOARD_VISIBLE), Some(false));
        assert_eq!(state.get_selected(SETUP_FIELD_TABS), Some("top"));
        assert_eq!(state.get_selected(SETUP_FIELD_ICON_BAR), Some("all"));
        // Separator Style acts on a console only, but is one global setting shown in
        // every interface (user ruling 2026-10-27).
        assert!(state.definition.get_field(SETUP_FIELD_SEPARATOR_STYLE).unwrap().visible, "Separator Style hidden in the console");
        assert_eq!(state.get_selected(SETUP_FIELD_SEPARATOR_STYLE), Some("tinyfugue"));
        let help = setup_help_text().join("\n");
        assert!(help.contains("Separator Style:"), "the console's help lacks Separator Style");
        for gone in ["GUI Theme:", "Keyboard Visible:", "Tabs:", "Icon Bar:", "Console only"] {
            assert!(!help.contains(gone), "the console's help still has {gone:?}");
        }
    }

    #[test]
    fn test_setup_popup_values() {
        let def = create_setup_popup(
            true, false, true, "alphabetical",
            true, 5, "light", true, "left", true, true,
            "background", "edge", "sentences", "bm_george", true, false, 4, false, "top", "all", "sxcu+short", true,
            "fast", "tinyfugue",
        );
        let state = PopupState::new(def);

        assert_eq!(state.get_selected(SETUP_FIELD_TTS), Some("edge"));
        assert_eq!(state.get_text(SETUP_FIELD_TTS_VOICE), Some("bm_george"));
        assert_eq!(state.get_bool(SETUP_FIELD_MORE_MODE), Some(true));
        assert_eq!(state.get_bool(SETUP_FIELD_SPELL_CHECK), Some(false));
        assert_eq!(state.get_bool(SETUP_FIELD_TEMP_CONVERT), Some(true));
        assert_eq!(state.get_selected(SETUP_FIELD_WORLD_SWITCHING), Some("alphabetical"));
        assert_eq!(state.get_bool(SETUP_FIELD_DEBUG), Some(true));
        assert_eq!(state.get_number(SETUP_FIELD_INPUT_HEIGHT), Some(5));
        assert_eq!(state.get_selected(SETUP_FIELD_GUI_THEME), Some("light"));
        assert_eq!(state.get_bool(SETUP_FIELD_TLS_PROXY), Some(true));
        assert_eq!(state.get_bool(SETUP_FIELD_ZWJ), Some(true));
        assert_eq!(state.get_number(SETUP_FIELD_WRAPSPACE), Some(4));
        assert_eq!(state.get_bool(SETUP_FIELD_KEYBOARD_VISIBLE), Some(false));
        assert_eq!(state.get_selected(SETUP_FIELD_TABS), Some("top"));
        assert_eq!(state.get_selected(SETUP_FIELD_ICON_BAR), Some("all"));
        assert_eq!(state.get_selected(SETUP_FIELD_PICTURE_HOST), Some("sxcu+short"));
        assert_eq!(state.get_bool(SETUP_FIELD_CLEAR_INPUT), Some(true));
        assert_eq!(state.get_selected(SETUP_FIELD_TEXT_REVEAL), Some("fast"));
        assert_eq!(state.get_selected(SETUP_FIELD_SEPARATOR_STYLE), Some("tinyfugue"));
    }

    /// New Indicator is a select of `NewIndicator::OPTIONS`, None first (the default); its
    /// value is read back by name, and an unknown saved value shows as None.
    #[test]
    fn the_new_indicator_choices_are_the_enums_options_none_first() {
        let values: Vec<String> = new_indicator_options("none").iter().map(|o| o.value.clone()).collect();
        assert_eq!(values, ["none", "character", "background", "custom:teal"]);
        let custom = new_indicator_options("custom:red");
        assert_eq!((custom[3].value.as_str(), custom[3].label.as_str()), ("custom:red", "Custom (Red)"));
        let labels: Vec<String> = new_indicator_options("none").iter().map(|o| o.label.clone()).collect();
        assert_eq!(labels, ["None", "Character", "Background", "Custom (Teal)"], "one word each (user request)");
        assert_eq!(new_indicator_index("custom:red"), 3);
        assert_eq!(crate::NewIndicator::default().name(), "none");
        let def = create_setup_popup_for_test("off");
        assert_eq!(PopupState::new(def).get_selected(SETUP_FIELD_NEW_LINE_INDICATOR), Some("none"));
        let def = create_setup_popup(
            true, false, true, "alphabetical",
            true, 5, "light", true, "left", true, true,
            "background", "edge", "sentences", "bm_george", true, false, 4, false, "top", "all", "sxcu+short", true,
            "fast", "tinyfugue",
        );
        assert_eq!(PopupState::new(def).get_selected(SETUP_FIELD_NEW_LINE_INDICATOR), Some("background"));
        let def = create_setup_popup(
            true, false, true, "alphabetical",
            true, 5, "light", true, "left", true, true,
            "plaid", "edge", "sentences", "bm_george", true, false, 4, false, "top", "all", "sxcu+short", true,
            "fast", "tinyfugue",
        );
        assert_eq!(PopupState::new(def).get_selected(SETUP_FIELD_NEW_LINE_INDICATOR), Some("none"));
        let help = setup_help_text().join("\n");
        assert!(help.contains("Background:") && help.contains("Character:") && help.contains("Custom:"), "the help names every style");
    }

    /// Kokoro is offered, and its voice shown, only where the build can load it (not the
    /// static musl console); a Kokoro setting there reads as Local, the engine that spoke.
    #[test]
    fn kokoro_is_offered_only_where_it_can_speak() {
        let values: Vec<String> = tts_mode_options().iter().map(|o| o.value.clone()).collect();
        assert_eq!(values.iter().any(|v| v == "kokoro"), crate::kokoro::build_can_load());
        let local = values.iter().position(|v| v == "local").unwrap();
        let kokoro_at = tts_mode_index("kokoro");
        if crate::kokoro::build_can_load() {
            assert_eq!(values[kokoro_at], "kokoro");
        } else {
            assert_eq!(kokoro_at, local);
        }
        assert_eq!(tts_mode_index("edge"), values.iter().position(|v| v == "edge").unwrap());
        let def = create_setup_popup_for_test("off");
        let voice = def.get_field(SETUP_FIELD_TTS_VOICE).unwrap();
        assert_eq!(voice.visible, crate::kokoro::build_can_load());
    }

    /// The Separator Style choices are `SeparatorStyle::OPTIONS`, Clay first (the default),
    /// and an unknown saved value shows as Web.
    #[test]
    fn the_separator_style_choices_are_the_enums_options_web_first() {
        let values: Vec<String> = separator_style_options().iter().map(|o| o.value.clone()).collect();
        assert_eq!(values, ["web", "tinyfugue"]);
        assert_eq!(crate::SeparatorStyle::default().name(), "web");
        let def = create_setup_popup_for_test("off");
        assert_eq!(PopupState::new(def).get_selected(SETUP_FIELD_SEPARATOR_STYLE), Some("web"));
    }
}
