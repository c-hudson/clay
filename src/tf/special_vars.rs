//! TF's special variables (`/help special variables`).
//!
//! TF gives a fixed set of global variable names special meaning and a type. Flags
//! and other enumerated variables accept only their listed values (by name, any case,
//! or by number) and are stored by name; in an expression they are dual-valued - the
//! name when printed, the number in arithmetic and as a truth value (`/set more=off`,
//! then `$[more]` is "off" but `$[!more]` is 1). Numeric variables take the number at
//! the start of whatever they are given. Everything here was checked against real tf.
//!
//! Assignments a user makes (`/set`, `:=`, `++`, `--`, `/toggle`) go through
//! `normalize`; values Clay itself mirrors into the engine (`insert`, `kbnum`) do not.

use super::TfValue;

/// What a special variable accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An enumerated variable: one of these names, index = its number. Flags are
    /// `["off", "on"]`.
    Enum(&'static [&'static str]),
    /// An integer.
    Int,
    /// A duration: seconds, or h:m[:s], kept as written.
    Dtime,
    /// Any text.
    Str,
}

const FLAG: Kind = Kind::Enum(&["off", "on"]);

/// One special variable: its name, kind, and the TF default the engine seeds at
/// startup (None when it is not seeded - left to the environment, or mirrored from or
/// left to Clay's own setting).
#[derive(Debug, Clone, Copy)]
pub struct SpecialVar {
    pub name: &'static str,
    pub kind: Kind,
    pub seed: Option<&'static str>,
}

const fn v(name: &'static str, kind: Kind, seed: Option<&'static str>) -> SpecialVar {
    SpecialVar { name, kind, seed }
}

/// TF 5.0b8's special variables. Seeds are TF's defaults wherever seeding cannot change
/// what Clay does by default; `more`, `wrapspace`, `isize`, `insert`, `wrap`, `wrapsize`,
/// `textdiv`, `lp` and `prompt_wait` are deliberately unseeded (Clay's own settings stand
/// for them). `visual` is seeded on and stays on: Clay's console has no non-visual mode.
/// The status-line variables (and `clock_format`, which only TF's status line reads) are
/// seeded: Clay shows TF's status line only once a script changes it
/// (`status::StatusLayout::customized`).
pub static SPECIAL_VARS: &[SpecialVar] = &[
    v("alert_attr", Kind::Str, Some("Br")),
    v("alert_time", Kind::Dtime, Some("5.0")),
    v("background", FLAG, Some("on")),
    v("backslash", FLAG, Some("on")),
    v("bamf", Kind::Enum(&["off", "on", "old"]), Some("off")),
    v("bg_output", FLAG, Some("on")),
    v("binary_eol", Kind::Enum(&["LF", "CR", "CRLF"]), Some("LF")),
    v("borg", FLAG, Some("on")),
    v("cleardone", FLAG, Some("off")),
    v("clearfull", FLAG, Some("off")),
    v("clock_format", Kind::Str, Some("%H:%M")),
    v("connect", Kind::Enum(&["blocking", "nonblocking"]), Some("nonblocking")),
    v("defcompile", FLAG, Some("off")),
    v("emulation", Kind::Enum(&["raw", "print", "ansi_strip", "ansi_attr", "debug"]), Some("ansi_attr")),
    v("end_color", Kind::Str, None),
    v("error_attr", Kind::Str, None),
    v("expand_tabs", FLAG, Some("on")),
    v("gag", FLAG, Some("on")),
    v("gethostbyname", Kind::Enum(&["blocking", "nonblocking"]), Some("nonblocking")),
    v("gpri", Kind::Int, Some("0")),
    v("hilite", FLAG, Some("on")),
    v("hiliteattr", Kind::Str, Some("B")),
    v("histsize", Kind::Int, Some("1000")),
    v("hook", FLAG, Some("on")),
    v("hpri", Kind::Int, Some("0")),
    v("insert", FLAG, None),
    v("interactive", FLAG, None),
    v("isize", Kind::Int, None),
    v("istrip", FLAG, Some("off")),
    v("kecho", FLAG, Some("off")),
    v("kecho_attr", Kind::Str, None),
    v("keepalive", FLAG, Some("on")),
    v("keypad", FLAG, Some("on")),
    v("kprefix", Kind::Str, None),
    v("login", FLAG, Some("on")),
    v("lp", FLAG, None),
    v("lpquote", FLAG, Some("off")),
    v("maildelay", Kind::Dtime, Some("0:01:00.0")),
    v("matching", Kind::Enum(&["simple", "glob", "regexp", "substr"]), Some("glob")),
    v("max_hook", Kind::Int, Some("1000")),
    v("max_instr", Kind::Int, Some("1000000")),
    v("max_kbnum", Kind::Int, Some("999")),
    v("max_recur", Kind::Int, Some("100")),
    v("max_trig", Kind::Int, Some("1000")),
    v("mccp", FLAG, Some("on")),
    v("mecho", Kind::Enum(&["off", "on", "all"]), Some("off")),
    v("mecho_attr", Kind::Str, None),
    v("meta_esc", Kind::Enum(&["off", "on", "nonprint"]), Some("nonprint")),
    v("more", FLAG, None),
    v("mprefix", Kind::Str, Some("+")),
    v("oldslash", FLAG, Some("on")),
    v("pedantic", FLAG, Some("off")),
    v("prompt_wait", Kind::Dtime, None),
    v("proxy_host", Kind::Str, None),
    v("proxy_port", Kind::Str, None),
    v("ptime", Kind::Dtime, Some("1.0")),
    v("qecho", FLAG, Some("off")),
    v("qecho_attr", Kind::Str, None),
    v("qprefix", Kind::Str, None),
    v("quiet", FLAG, Some("off")),
    v("quitdone", FLAG, Some("off")),
    v("redef", FLAG, Some("on")),
    v("refreshtime", Kind::Int, Some("100000")),
    v("scroll", FLAG, Some("on")),
    v("secho", FLAG, Some("off")),
    v("secho_attr", Kind::Str, None),
    v("shpause", FLAG, Some("on")),
    v("sigfigs", Kind::Int, Some("15")),
    v("snarf", FLAG, Some("off")),
    v("sockmload", FLAG, Some("off")),
    v("sprefix", Kind::Str, None),
    v("status_attr", Kind::Str, None),
    v("status_height", Kind::Int, Some("1")),
    v("status_pad", Kind::Str, Some("_")),
    v("sub", Kind::Enum(&["off", "on", "full"]), Some("off")),
    v("tabsize", Kind::Int, Some("8")),
    v("telopt", FLAG, Some("off")),
    v("textdiv", Kind::Enum(&["off", "on", "always", "clear"]), None),
    v("textdiv_str", Kind::Str, Some("=====")),
    v("time_format", Kind::Str, Some("%H:%M")),
    v("visual", FLAG, Some("on")),
    v("warn_5keys", FLAG, Some("on")),
    v("warn_curly_re", FLAG, Some("on")),
    v("warn_status", FLAG, Some("on")),
    v("warning_attr", Kind::Str, None),
    v("watchdog", FLAG, None),
    v("watchname", FLAG, None),
    v("wordpunct", Kind::Str, Some("_")),
    v("wrap", FLAG, None),
    v("wraplog", FLAG, Some("off")),
    v("wrappunct", Kind::Int, Some("10")),
    v("wrapsize", Kind::Int, None),
    v("wrapspace", Kind::Int, None),
];

/// Special variables that stand for one of Clay's own settings: the App mirrors the
/// setting into them (never saved as TF variables) and applies a user's assignment to it
/// (`TfEffect::Setting`). `%more` is Clay's more-paging, `%wrapspace` its wrap indent,
/// `%isize` the input area's height, `%insert` insert vs. overwrite typing.
pub const BOUND: &[&str] = &["more", "wrapspace", "isize", "insert"];

/// Variables whose assignment the App applies too, but which keep their TF meaning rather
/// than mirroring a Clay setting: `%wrap`/`%wrapsize` (unset, Clay wraps at the window's
/// edge; set, at %wrapsize), `%clock_format` (unset, Clay's 12-hour clock) and
/// `%textdiv`/`%textdiv_str` (unset, Clay's own ▶ new-text markers). Unsetting one is
/// applied too (`TfEngine::unset_global`).
pub const APPLIED: &[&str] = &["wrap", "wrapsize", "clock_format", "textdiv", "textdiv_str"];

/// The special variable called `name`, if it is one.
pub fn lookup(name: &str) -> Option<&'static SpecialVar> {
    SPECIAL_VARS.iter().find(|v| v.name == name)
}

/// The value a user assignment to special variable `var` actually stores, or TF's own
/// error message when the value is not allowed (the old value is then kept).
pub fn normalize(var: &SpecialVar, value: &TfValue) -> Result<TfValue, String> {
    let normalized = normalize_kind(var, value)?;
    // TF's non-visual mode is not offered: Clay's console always draws its windows.
    if var.name == "visual" && !flag_is_on(&normalized) {
        return Err(NO_NONVISUAL_MODE.to_string());
    }
    Ok(normalized)
}

/// What `/visual off` (or `/set visual=off`) answers.
pub const NO_NONVISUAL_MODE: &str = "Clay's console has no non-visual mode.";

fn normalize_kind(var: &SpecialVar, value: &TfValue) -> Result<TfValue, String> {
    match var.kind {
        Kind::Enum(names) => {
            let text = value.to_string_value();
            let trimmed = text.trim();
            if let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(trimmed)) {
                return Ok(TfValue::String(name.to_string()));
            }
            if let Ok(n) = trimmed.parse::<i64>() {
                if n >= 0 && (n as usize) < names.len() {
                    return Ok(TfValue::String(names[n as usize].to_string()));
                }
            }
            let valid: Vec<String> = names.iter().enumerate().map(|(i, n)| format!("{} ({})", n, i)).collect();
            Err(format!("Invalid {} value \"{}\".  Valid values are: {}", var.name, text, valid.join(", ")))
        }
        Kind::Int => {
            let n = value.tf_int();
            if var.name == "isize" && n < 1 {
                return Err("isize must be an integer greater than or equal to 1".to_string());
            }
            Ok(TfValue::Integer(n))
        }
        Kind::Dtime | Kind::Str => Ok(value.clone()),
    }
}

/// How special variable `name`, stored as `stored`, reads in an expression: an
/// enumerated variable is dual-valued (`TfValue::Enum`), anything else reads as stored.
pub fn expr_value(name: &str, stored: &TfValue) -> TfValue {
    if let Some(SpecialVar { kind: Kind::Enum(names), .. }) = lookup(name) {
        let text = stored.to_string_value();
        if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(&text)) {
            return TfValue::Enum(i as i64, names[i].to_string());
        }
    }
    stored.clone()
}

/// A TF "dtime" value in seconds: "1.5", or "h:m" / "h:m:s" (`/help dtime`).
pub fn dtime_secs(text: &str) -> Option<f64> {
    let parts: Vec<f64> = text.trim().split(':').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let secs = match parts.as_slice() {
        [s] => *s,
        [h, m] => h * 3600.0 + m * 60.0,
        [h, m, s] => h * 3600.0 + m * 60.0 + s,
        _ => return None,
    };
    (secs.is_finite() && secs >= 0.0).then_some(secs)
}

/// A flag's truth value, by its stored name ("on"/"off") or number.
pub fn flag_is_on(stored: &TfValue) -> bool {
    let text = stored.to_string_value();
    if text.eq_ignore_ascii_case("on") {
        return true;
    }
    if text.eq_ignore_ascii_case("off") {
        return false;
    }
    stored.to_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_enums_ints_and_errors() {
        let more = lookup("more").unwrap();
        assert_eq!(normalize(more, &TfValue::Integer(1)).unwrap(), TfValue::String("on".into()));
        assert_eq!(normalize(more, &TfValue::String("ON".into())).unwrap(), TfValue::String("on".into()));
        assert_eq!(
            normalize(more, &TfValue::String("foo".into())).unwrap_err(),
            "Invalid more value \"foo\".  Valid values are: off (0), on (1)"
        );
        let matching = lookup("matching").unwrap();
        assert_eq!(normalize(matching, &TfValue::Integer(2)).unwrap(), TfValue::String("regexp".into()));
        let wrapspace = lookup("wrapspace").unwrap();
        assert_eq!(normalize(wrapspace, &TfValue::String("7abc".into())).unwrap(), TfValue::Integer(7));
        assert_eq!(normalize(wrapspace, &TfValue::String("abc".into())).unwrap(), TfValue::Integer(0));
        assert!(normalize(lookup("isize").unwrap(), &TfValue::String("x".into())).is_err());
        let ptime = lookup("ptime").unwrap();
        assert_eq!(normalize(ptime, &TfValue::String("0:01".into())).unwrap(), TfValue::String("0:01".into()));
    }

    /// Clay's console is always visual: `/visual off` and `/set visual=off` are refused
    /// with Clay's own message, and %visual keeps reading "on".
    #[test]
    fn test_visual_off_is_refused() {
        let visual = lookup("visual").unwrap();
        assert_eq!(normalize(visual, &TfValue::String("on".into())).unwrap(), TfValue::String("on".into()));
        assert_eq!(normalize(visual, &TfValue::Integer(0)).unwrap_err(), NO_NONVISUAL_MODE);
        let mut engine = crate::tf::TfEngine::new();
        for command in ["/visual off", "/set visual=off", "/set visual 0"] {
            let result = crate::tf::parser::execute_command(&mut engine, command);
            assert!(matches!(&result, crate::tf::TfCommandResult::Error(e) if e == NO_NONVISUAL_MODE),
                "{}: {:?}", command, result);
            assert_eq!(engine.get_var("visual").map(|v| v.to_string_value()).as_deref(), Some("on"));
        }
        assert!(!BOUND.contains(&"visual"), "nothing in Clay follows %visual any more");
    }

    #[test]
    fn test_expr_value_is_dual_for_enums() {
        assert_eq!(expr_value("more", &TfValue::String("off".into())), TfValue::Enum(0, "off".into()));
        assert_eq!(expr_value("matching", &TfValue::String("regexp".into())), TfValue::Enum(2, "regexp".into()));
        assert_eq!(expr_value("myvar", &TfValue::String("off".into())), TfValue::String("off".into()));
        assert!(flag_is_on(&TfValue::String("on".into())));
        assert!(!flag_is_on(&TfValue::String("off".into())));
        assert!(flag_is_on(&TfValue::Integer(1)));
    }
}
