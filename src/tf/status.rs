//! TF's status area (`/help status area`; checked against real tf 5.0b8): rows of
//! fields, each `name[:width[:attributes]]`, edited by /status_add, /status_rm,
//! /status_edit, /status_defaults, /status_save, /status_restore and /clock. A field's
//! text comes from the expression in %status_int_<name> (an `@<name>` internal state)
//! or %status_var_<name> (a variable; else the variable's value), or is a "literal".
//!
//! Clay draws its own bar until a script changes the status area
//! (`StatusLayout::customized`); from then on it draws this one, in every interface:
//! `evaluate` turns the fields into text for a world, and each interface lays the
//! rows out to its own width with `layout_row` (app.js has a port of it).

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use super::{TfAttributes, TfCommandResult, TfEngine, TfValue};

/// TF's row 0 (tfstatus.tf's %status_field_defaults).
pub const DEFAULT_FIELDS: &str = "@more:8:Br :1 @world :1 @read:6 :1 @active:11 :1 @log:5 :1 @mail:6 :1 insert:6 :1 kbnum:4 :1 @clock:5";

/// tfstatus.tf's formats: each is seeded as %status_<kind>_<name> and as the
/// %status_std_<kind>_<name> that /status_defaults restores it from.
const STD_FORMATS: &[(&str, &str)] = &[
    ("int_more", r#"(limit() | morepaused()) ? status_int_more() : """#),
    ("int_world", r#"fg_world() =~ "" ? "(no world)" : strcat(!is_open(fg_world()) ? "!" : "",  fg_world())"#),
    ("int_read", r#"nread() ? "(Read)" : """#),
    ("int_active", r#"nactive() ? pad("(Active:", 0, nactive(), 2, ")") : """#),
    ("int_log", r#"nlog() ? "(Log)" : """#),
    ("int_mail", r#"!nmail() ? "" : nmail()==1 ? "(Mail)" : pad("Mail", 0, nmail(), 2)"#),
    ("var_insert", r#"insert ? "" : "(Over)""#),
    ("int_clock", "ftime(clock_format)"),
];

/// tfstatus.tf's `status_int_more` macro, which %status_int_more calls; Clay has it as a
/// function instead (a macro of that name, loaded from tfstatus.tf, still wins). Its
/// "old" lines - seen, then scrolled back below the window - don't exist in Clay, whose
/// waiting lines are all new.
pub fn status_int_more(waiting: usize, limit: bool, paused: bool) -> String {
    if limit && !paused {
        return "LIMIT ON".to_string();
    }
    let count = if waiting >= 10000 { format!("{}k", waiting / 1000) } else { waiting.to_string() };
    format!("{:>4}{:>4}", if limit { "LIM " } else { "More" }, count)
}

/// One status field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatusField {
    /// "" for padding, "@<state>" for an internal state, a variable's name, or a
    /// literal kept with its double quotes ("\"text\"", `\"` inside it escaped).
    pub name: String,
    /// Columns, when given; none (or 0) is the row's one variable-width field - except
    /// for a literal, whose width is then its length.
    pub width: Option<u16>,
    /// Right-justified: a negative width, or "-0".
    pub right: bool,
    /// The attributes given, in TF's order.
    pub attrs: String,
}

impl StatusField {
    /// Parse one `name[:width[:attributes]]`.
    pub fn parse(spec: &str) -> Result<StatusField, String> {
        let garbage = |rest: &str| format!("garbage in status field: {}", rest);
        let (name, rest) = match spec.chars().next() {
            Some(q @ ('"' | '\'' | '`')) => {
                let mut text = String::new();
                let mut chars = spec.char_indices().skip(1);
                let mut end = None;
                while let Some((i, c)) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some((_, next)) = chars.next() {
                                text.push(next);
                            }
                        }
                        c if c == q => {
                            end = Some(i + c.len_utf8());
                            break;
                        }
                        c => text.push(c),
                    }
                }
                let end = end.ok_or_else(|| garbage(spec))?;
                (format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")), &spec[end..])
            }
            _ => {
                let end = spec.find(':').unwrap_or(spec.len());
                (spec[..end].to_string(), &spec[end..])
            }
        };
        let mut field = StatusField { name, ..Default::default() };
        if rest.is_empty() {
            return Ok(field);
        }
        let Some(rest) = rest.strip_prefix(':') else {
            return Err(garbage(rest));
        };
        let (width, attrs) = match rest.find(':') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        if !width.is_empty() {
            let (right, digits) = match width.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, width),
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return Err(garbage(width));
            }
            let n: u16 = digits.parse().map_err(|_| garbage(width))?;
            field.right = right;
            field.width = (n > 0 || field.is_literal()).then_some(n);
        }
        if let Some(attrs) = attrs {
            field.attrs = TfAttributes::parse(attrs)?.canonical();
        }
        Ok(field)
    }

    /// As status_fields() shows it.
    pub fn spec(&self) -> String {
        let mut out = self.name.clone();
        let width = match (self.width, self.right) {
            (Some(w), true) => format!("-{}", w),
            (Some(w), false) => w.to_string(),
            (None, true) => "-0".to_string(),
            (None, false) => String::new(),
        };
        if !width.is_empty() || !self.attrs.is_empty() {
            out.push(':');
            out.push_str(&width);
        }
        if !self.attrs.is_empty() {
            out.push(':');
            out.push_str(&self.attrs);
        }
        out
    }

    pub fn is_pad(&self) -> bool {
        self.name.is_empty()
    }

    fn is_literal(&self) -> bool {
        self.name.starts_with('"')
    }

    /// The text of a literal field.
    fn literal(&self) -> Option<String> {
        let inner = self.name.strip_prefix('"')?.strip_suffix('"')?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.extend(chars.next()),
                c => out.push(c),
            }
        }
        Some(out)
    }

    /// Takes whatever columns the other fields leave.
    fn is_flexible(&self) -> bool {
        self.width.is_none() && !self.is_literal()
    }

    /// The columns a fixed field takes.
    fn columns(&self) -> u16 {
        match self.width {
            Some(w) => w,
            None => self.literal().map_or(0, |t| text_columns(&t) as u16),
        }
    }
}

/// The status area's fields, row by row (rows past %status_height are kept, not shown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatusLayout {
    pub rows: Vec<Vec<StatusField>>,
    /// A script has changed the status area (a status command, or setting one of its
    /// variables): from then on it is drawn in place of Clay's own bar.
    pub customized: bool,
}

impl StatusLayout {
    /// TF's: its default row 0.
    pub fn tf_default() -> StatusLayout {
        StatusLayout { rows: vec![parse_fields(DEFAULT_FIELDS).unwrap_or_default()], customized: false }
    }

    fn row_mut(&mut self, row: usize) -> &mut Vec<StatusField> {
        if self.rows.len() <= row {
            self.rows.resize(row + 1, Vec::new());
        }
        &mut self.rows[row]
    }

    /// status_fields(row).
    pub fn fields_text(&self, row: usize) -> String {
        self.rows.get(row).map(|r| r.iter().map(StatusField::spec).collect::<Vec<_>>().join(" ")).unwrap_or_default()
    }
}

fn parse_fields(text: &str) -> Result<Vec<StatusField>, String> {
    split_specs(text).iter().map(|s| StatusField::parse(s)).filter(|f| !matches!(f, Ok(f) if f.is_pad() && f.width.is_none())).collect()
}

/// Field specs are separated by spaces, which a quoted literal may contain.
fn split_specs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') if quote.is_some() => {
                cur.push(c);
                cur.extend(chars.next());
            }
            (Some(q), c) if c == q => {
                cur.push(c);
                quote = None;
            }
            (None, '"' | '\'' | '`') if cur.is_empty() => {
                cur.push(c);
                quote = Some(c);
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// More than one variable-width field in a row.
fn too_flexible(row: &[StatusField]) -> bool {
    row.iter().filter(|f| f.is_flexible() && !f.is_pad()).count() > 1
}

/// Seed TF's status area into a new engine: the default layout and tfstatus.tf's
/// formats (none of it customizes the area).
pub fn seed(engine: &mut TfEngine) {
    engine.status = StatusLayout::tf_default();
    for (name, format) in STD_FORMATS {
        engine.set_global(&format!("status_{}", name), TfValue::String(format.to_string()));
        engine.set_global(&format!("status_std_{}", name), TfValue::String(format.to_string()));
    }
    engine.set_global("status_field_defaults", TfValue::String(DEFAULT_FIELDS.to_string()));
    engine.set_global("std_clock_format", TfValue::String("%H:%M".to_string()));
    sync_fields_var(engine);
}

/// The names this module seeds (never saved while unchanged).
pub fn seeded_names() -> Vec<String> {
    let mut names: Vec<String> = STD_FORMATS.iter()
        .flat_map(|(n, _)| [format!("status_{}", n), format!("status_std_{}", n)])
        .collect();
    names.extend(["status_field_defaults", "std_clock_format", "status_fields"].iter().map(|s| s.to_string()));
    names
}

/// %status_fields mirrors row 0 (TF keeps it for older scripts).
fn sync_fields_var(engine: &mut TfEngine) {
    let text = engine.status.fields_text(0);
    engine.set_global("status_fields", TfValue::String(text));
}

/// A variable that is part of the status area was assigned: the area is now the
/// script's (`StatusLayout::customized`). `/set status_fields=...` sets row 0, with TF's
/// warning unless %warn_status is off. Returns an error to report, if any.
pub fn on_assign(engine: &mut TfEngine, name: &str, value: &str) -> Option<String> {
    if name == "status_fields" {
        let fields = match parse_fields(value) {
            Ok(fields) if !too_flexible(&fields) => fields,
            Ok(_) => return Some("Only one variable width status field is allowed.".to_string()),
            Err(e) => return Some(e),
        };
        *engine.status.row_mut(0) = fields;
        changed(engine);
        if engine.get_var("warn_status").is_none_or(super::special_vars::flag_is_on) {
            engine.emit(super::effects::TfEffect::error(
                "Warning: setting status_fields directly is deprecated, and may clobber useful new features introduced in version 5.  The recommended way to change status fields is with /status_add, /status_rm, or /status_edit. (This warning can be disabled with \"/set warn_status=off\".)"));
        }
        return None;
    }
    if name.starts_with("status_") && !name.starts_with("status_std_") && name != "status_field_defaults" {
        engine.status.customized = true;
    }
    None
}

/// `-r<N>` and the rest of a status command's arguments.
fn row_option(args: &str) -> (Option<usize>, &str) {
    let args = args.trim_start();
    if let Some(rest) = args.strip_prefix("-r") {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if let Ok(n) = rest[..end].parse::<usize>() {
            return (Some(n), rest[end..].trim_start());
        }
    }
    (None, args)
}

/// `/status_add [-r<N>] [-A[<field>]] [-B[<field>]] [-s<N>] [-x] [-c] <field>...`
pub fn cmd_status_add(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let mut rest = args.trim_start();
    let mut row = 0usize;
    let mut before: Option<String> = None; // Some("") = at the beginning
    let mut after: Option<String> = None;
    let mut spacing = 1usize;
    let mut unique = false;
    let mut clear = false;
    while let Some(opt) = rest.strip_prefix('-') {
        let end = opt.find(char::is_whitespace).unwrap_or(opt.len());
        let word = &opt[..end];
        rest = opt[end..].trim_start();
        if word.is_empty() || word == "-" {
            break;
        }
        let (flag, value) = word.split_at(1);
        match flag {
            "r" => match value.parse() {
                Ok(n) => row = n,
                Err(_) => return TfCommandResult::Error(format!("STATUS_ADD -r: invalid row {}", value)),
            },
            "A" => after = Some(strip_quotes(value)),
            "B" => before = Some(strip_quotes(value)),
            "s" => match value.parse() {
                Ok(n) => spacing = n,
                Err(_) => return TfCommandResult::Error(format!("STATUS_ADD -s: invalid spacing {}", value)),
            },
            "x" if value.is_empty() => unique = true,
            "c" if value.is_empty() => clear = true,
            _ => return TfCommandResult::Error(format!("STATUS_ADD -{}: invalid option", flag)),
        }
    }
    let fields = match parse_fields(rest) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(format!("STATUS_ADD: {}", e)),
    };
    let mut new_row = engine.status.rows.get(row).cloned().unwrap_or_default();
    if clear {
        new_row.clear();
    }
    let fields: Vec<StatusField> = fields.into_iter()
        .filter(|f| !(unique && !f.is_pad() && new_row.iter().any(|g| g.name == f.name)))
        .collect();
    if fields.is_empty() {
        return TfCommandResult::Success(None);
    }
    let pad = (spacing > 0 && !new_row.is_empty())
        .then(|| StatusField { width: Some(spacing as u16), ..Default::default() });
    let position = match (&before, &after) {
        (Some(name), _) if !name.is_empty() => match new_row.iter().position(|f| &f.name == name) {
            Some(i) => Some((i, true)),
            None => return TfCommandResult::Error(format!("STATUS_ADD: {}: no such field", name)),
        },
        (Some(_), _) => Some((0, true)),
        (None, Some(name)) if !name.is_empty() => match new_row.iter().position(|f| &f.name == name) {
            Some(i) => Some((i + 1, false)),
            None => return TfCommandResult::Error(format!("STATUS_ADD: {}: no such field", name)),
        },
        _ => None,
    };
    let (at, is_before) = position.unwrap_or((new_row.len(), false));
    let mut insert = Vec::new();
    if !is_before {
        insert.extend(pad.clone());
    }
    insert.extend(fields);
    if is_before {
        insert.extend(pad);
    }
    new_row.splice(at..at, insert);
    if too_flexible(&new_row) {
        return TfCommandResult::Error("STATUS_ADD: Only one variable width status field is allowed.".to_string());
    }
    *engine.status.row_mut(row) = new_row;
    changed(engine);
    TfCommandResult::Success(None)
}

fn strip_quotes(name: &str) -> String {
    let name = name.trim();
    for q in ['"', '\'', '`'] {
        if let Some(inner) = name.strip_prefix(q).and_then(|n| n.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    name.to_string()
}

/// `/status_rm [-r<N>] <name>`: the first such field goes, with the smaller of the pads
/// beside it (or the one pad beside a field at either end).
pub fn cmd_status_rm(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let (row, rest) = row_option(args);
    let name = rest.trim();
    let found = engine.status.rows.iter().enumerate()
        .filter(|(r, _)| row.is_none_or(|want| want == *r))
        .find_map(|(r, fields)| fields.iter().position(|f| f.name == name).map(|i| (r, i)));
    let Some((r, i)) = found else {
        return TfCommandResult::Error(match row {
            Some(n) => format!("STATUS_RM: {}: no such field in row {}", name, n),
            None => format!("STATUS_RM: {}: no such field", name),
        });
    };
    let fields = &mut engine.status.rows[r];
    fields.remove(i);
    let left = i.checked_sub(1).filter(|&j| fields[j].is_pad());
    let right = (i < fields.len() && fields[i].is_pad()).then_some(i);
    let drop = match (left, right) {
        (Some(l), Some(r)) => Some(if fields[r].columns() <= fields[l].columns() { r } else { l }),
        (Some(l), None) if i == fields.len() => Some(l),
        (None, Some(r)) if i == 0 => Some(r),
        _ => None,
    };
    if let Some(j) = drop {
        fields.remove(j);
    }
    changed(engine);
    TfCommandResult::Success(None)
}

/// `/status_edit [-r<N>] <name>[:<width>[:<attributes>]]`: replace the first field of
/// that name; the padding around it stays.
pub fn cmd_status_edit(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let (row, rest) = row_option(args);
    let spec = rest.trim();
    let field = match StatusField::parse(spec) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(format!("STATUS_EDIT: {}", e)),
    };
    let found = engine.status.rows.iter().enumerate()
        .filter(|(r, _)| row.is_none_or(|want| want == *r))
        .find_map(|(r, fields)| fields.iter().position(|f| f.name == field.name).map(|i| (r, i)));
    let Some((r, i)) = found else {
        return TfCommandResult::Error(match row {
            Some(n) => format!("STATUS_EDIT: {}: no such field in row {}", spec, n),
            None => format!("STATUS_EDIT: {}: no such field", spec),
        });
    };
    let mut new_row = engine.status.rows[r].clone();
    new_row[i] = field;
    if too_flexible(&new_row) {
        return TfCommandResult::Error("STATUS_EDIT: Only one variable width status field per row is allowed.".to_string());
    }
    engine.status.rows[r] = new_row;
    changed(engine);
    TfCommandResult::Success(None)
}

/// `/status_defaults`: tfstatus.tf's - its formats and clock back, and row 0's fields.
pub fn cmd_status_defaults(engine: &mut TfEngine) -> TfCommandResult {
    let clock = engine.get_var("std_clock_format").map(|v| v.to_string_value()).unwrap_or_else(|| "%H:%M".to_string());
    let _ = engine.assign_global("clock_format", TfValue::String(clock));
    for (name, _) in STD_FORMATS {
        if let Some(value) = engine.get_var(&format!("status_std_{}", name)).cloned() {
            let _ = engine.assign_global(&format!("status_{}", name), value);
        }
    }
    let defaults = engine.get_var("status_field_defaults").map(|v| v.to_string_value())
        .unwrap_or_else(|| DEFAULT_FIELDS.to_string());
    cmd_status_add(engine, &format!("-c - {}", defaults))
}

/// `/status_save <name>`: row 0's fields, kept as %_status_save_<name>.
pub fn cmd_status_save(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.split_whitespace().next().unwrap_or("");
    let fields = engine.status.fields_text(0);
    let _ = engine.assign_global(&format!("_status_save_{}", name), TfValue::String(fields));
    TfCommandResult::Success(None)
}

/// `/status_restore <name>`: row 0 as `/status_save <name>` kept it.
pub fn cmd_status_restore(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.split_whitespace().next().unwrap_or("");
    match engine.get_var(&format!("_status_save_{}", name)).map(|v| v.to_string_value()) {
        Some(fields) => cmd_status_add(engine, &format!("-c - {}", fields)),
        None => {
            engine.emit(super::effects::TfEffect::error(format!("% No saved status \"{}\".", name)));
            TfCommandResult::Success(None)
        }
    }
}

/// `/clock [on|off|<format>]` (tfstatus.tf's): off removes @clock; otherwise a
/// %clock_format of <format> (when given) and an @clock just wide enough for it, added
/// at the end of row 0 unless it is there already.
pub fn cmd_clock(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim();
    if matches!(arg, "0" | "off" | "no") {
        return cmd_status_rm(engine, "@clock");
    }
    if !matches!(arg, "1" | "on" | "yes") {
        let format = if arg.is_empty() { "%H:%M" } else { arg };
        let _ = engine.assign_global("clock_format", TfValue::String(format.to_string()));
    }
    let format = engine.get_var("clock_format").map(|v| v.to_string_value()).unwrap_or_default();
    let width = text_columns(&ftime_now(&format));
    let existing = engine.status.rows.first().and_then(|r| r.iter().find(|f| f.name == "@clock")).cloned();
    match existing {
        Some(clock) => {
            let attrs = if clock.attrs.is_empty() { String::new() } else { format!(":{}", clock.attrs) };
            cmd_status_edit(engine, &format!("@clock:{}{}", width, attrs))
        }
        None => cmd_status_add(engine, &format!("-A -x @clock:{}", width)),
    }
}

/// `ftime(format)` for the time now.
fn ftime_now(format: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as i64;
    let lt = crate::util::local_time_from_epoch(secs);
    super::expressions::format_tf_time(&lt, secs, now.subsec_nanos() as f64 / 1e9, format)
}

/// The layout changed: it is the script's now, and %status_fields follows row 0.
fn changed(engine: &mut TfEngine) {
    engine.status.customized = true;
    sync_fields_var(engine);
}

/// One field as an interface shows it: its text in its attributes, and how it is laid
/// out (`layout_row`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatusCell {
    /// The text, attributes applied as SGR (empty for padding).
    pub text: String,
    /// Columns (the text is cut to fit); unused for the flexible field.
    pub width: u16,
    /// The field that takes whatever columns the others leave.
    pub flex: bool,
    /// Right-justified within its columns.
    pub right: bool,
}

/// The status area as an interface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatusView {
    pub rows: Vec<Vec<StatusCell>>,
    /// %status_pad: what fills a field to its width.
    pub pad: String,
    /// %status_attr, as SGR, over the whole area.
    pub attr: String,
}

/// The status area's rows (%status_height of them) as `world` would show them: each
/// field's format evaluated with `world` as the foreground and current world, as TF
/// does for the world whose state changed.
pub fn evaluate(engine: &mut TfEngine, world: Option<&str>) -> StatusView {
    let height = engine.get_var("status_height").and_then(|v| v.to_int()).unwrap_or(1).max(0) as usize;
    let saved_fg = engine.current_world.clone();
    if world.is_some() {
        engine.current_world = world.map(str::to_string);
    }
    engine.begin_frame();
    let rows = (0..height).map(|r| {
        let fields = engine.status.rows.get(r).cloned().unwrap_or_default();
        fields.iter().map(|f| cell(engine, world, f)).collect()
    }).collect();
    // What the formats did besides giving text (an echo(), say) is dropped.
    let _ = engine.end_frame();
    engine.current_world = saved_fg;
    let attr = engine.get_var("status_attr").map(|v| v.to_string_value()).unwrap_or_default();
    StatusView {
        rows,
        pad: engine.get_var("status_pad").map(|v| v.to_string_value()).unwrap_or_default(),
        attr: TfAttributes::parse(&attr).map(|a| a.to_sgr()).unwrap_or_default(),
    }
}

fn cell(engine: &mut TfEngine, world: Option<&str>, field: &StatusField) -> StatusCell {
    let (text, attr_var) = if field.is_pad() {
        (String::new(), None)
    } else if let Some(text) = field.literal() {
        (text, None)
    } else if let Some(state) = field.name.strip_prefix('@') {
        let text = format_var(engine, world, &format!("status_int_{}", state)).unwrap_or_default();
        (text, Some(format!("status_attr_int_{}", state)))
    } else {
        let var = &field.name;
        let text = format_var(engine, world, &format!("status_var_{}", var))
            .unwrap_or_else(|| engine.get_var(var).map(|v| v.to_string_value()).unwrap_or_default());
        (text, Some(format!("status_attr_var_{}", var)))
    };
    let mut attrs = TfAttributes::parse(&field.attrs).unwrap_or_default();
    if let Some(extra) = attr_var.and_then(|v| engine.get_var(&v).map(|v| v.to_string_value())) {
        if let Ok(extra) = TfAttributes::parse(&extra) {
            attrs.merge(&extra);
        }
    }
    let sgr = attrs.to_sgr();
    let text = if sgr.is_empty() || text.is_empty() { text } else { format!("{}{}\x1b[0m", sgr, text) };
    StatusCell { text, width: field.columns(), flex: field.is_flexible(), right: field.right }
}

/// The value of the expression in format variable `var`, if it is set: as text, the
/// way TF shows it (errors show nothing).
fn format_var(engine: &mut TfEngine, world: Option<&str>, var: &str) -> Option<String> {
    let expr = engine.get_var(var)?.to_string_value();
    if expr.trim().is_empty() {
        return Some(String::new());
    }
    let value = engine.with_context_world(world, |engine| super::expressions::evaluate(engine, &expr));
    Some(value.map(|v| v.to_string_value()).unwrap_or_default())
}

/// Columns `text` takes on screen (escape sequences take none).
pub fn text_columns(text: &str) -> usize {
    crate::util::strip_ansi_codes(text).chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// `text` cut to `max` columns, escape sequences kept (all of them, so a closing reset
/// survives the cut); and how many columns it takes.
pub(crate) fn cut_to(text: &str, max: usize) -> (String, usize) {
    let mut out = String::new();
    let mut cols = 0;
    let mut cut = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // An escape sequence: kept whole, takes no columns.
            out.push(c);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap_or('['));
                for c in chars.by_ref() {
                    out.push(c);
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        let w = c.width().unwrap_or(0);
        if cut || cols + w > max {
            cut = true;
            continue;
        }
        out.push(c);
        cols += w;
    }
    (out, cols)
}

/// Lay one row out in `width` columns, as TF draws it: each fixed field takes its
/// width, the flexible one whatever is left, text cut to fit and filled out with `pad`
/// (right-justified where asked), the rest of the row filled with `pad` too, and all of
/// it cut at the edge; the whole row in `attr` (SGR).
pub fn layout_row(cells: &[StatusCell], width: usize, pad: &str, attr: &str) -> String {
    let pad_char = pad.chars().next().unwrap_or(' ');
    let fixed: usize = cells.iter().filter(|c| !c.flex).map(|c| c.width as usize).sum();
    let flex_width = width.saturating_sub(fixed);
    let mut out = String::from(attr);
    let mut used = 0;
    for cell in cells {
        if used >= width {
            break;
        }
        let columns = if cell.flex { flex_width } else { cell.width as usize }.min(width - used);
        let (text, text_cols) = cut_to(&cell.text, columns);
        let fill: String = std::iter::repeat_n(pad_char, columns - text_cols).collect();
        // The field's own attributes end with a reset: the row's come back after it.
        let resume = if text.contains('\x1b') { attr } else { "" };
        if cell.right {
            out.push_str(&fill);
            out.push_str(&text);
            out.push_str(resume);
        } else {
            out.push_str(&text);
            out.push_str(resume);
            out.push_str(&fill);
        }
        used += columns;
    }
    out.extend(std::iter::repeat_n(pad_char, width.saturating_sub(used)));
    if !attr.is_empty() {
        out.push_str("\x1b[0m");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(engine: &mut TfEngine, row: usize) -> String {
        engine.status.fields_text(row)
    }

    /// Field specs read and print back as real tf's status_fields() shows them.
    #[test]
    fn test_field_specs_round_trip_as_tf_prints_them() {
        let row = parse_fields(r#"a:-5 b:-0 :2:r "lit":4:B 'q' `bt`:3 "es\"c" @world:5:Cred,u x:03 a:0 'c"d'"#).unwrap();
        let text: Vec<String> = row.iter().map(StatusField::spec).collect();
        assert_eq!(text.join(" "), r#"a:-5 b:-0 :2:r "lit":4:B "q" "bt":3 "es\"c" @world:5:uCred x:3 a "c\"d""#);
        assert_eq!(StatusField::parse("@world:0:B").unwrap().spec(), "@world::B");
        assert_eq!(StatusField::parse("a:x").unwrap_err(), "garbage in status field: x");
        assert_eq!(StatusField::parse("a:3:Q").unwrap_err(), "invalid display attribute 'Q'");
        assert!(parse_fields(":0").unwrap().is_empty(), "a zero-width pad is nothing");
        let mut engine = TfEngine::new();
        assert_eq!(fields(&mut engine, 0), "@more:8:rB :1 @world :1 @read:6 :1 @active:11 :1 @log:5 :1 @mail:6 :1 insert:6 :1 kbnum:4 :1 @clock:5");
        assert!(!engine.status.customized);
    }

    /// /status_add, /status_rm and /status_edit do what real tf does to the fields.
    #[test]
    fn test_status_commands_edit_fields_as_tf() {
        let mut engine = TfEngine::new();
        let ok = |r: TfCommandResult| assert!(matches!(r, TfCommandResult::Success(None)), "{:?}", r);
        ok(cmd_status_add(&mut engine, "-A@world hp:4"));
        assert!(fields(&mut engine, 0).starts_with("@more:8:rB :1 @world :1 hp:4 :1 @read:6"));
        assert!(engine.status.customized);
        ok(cmd_status_add(&mut engine, "-B foo:3"));
        assert!(fields(&mut engine, 0).starts_with("foo:3 :1 @more:8:rB"));
        let before = fields(&mut engine, 0);
        ok(cmd_status_add(&mut engine, "-x hp:4"));
        assert_eq!(fields(&mut engine, 0), before, "-x: already there");
        ok(cmd_status_add(&mut engine, "hp:4"));
        assert!(fields(&mut engine, 0).ends_with("@clock:5 :1 hp:4"));
        ok(cmd_status_rm(&mut engine, "@mail"));
        assert!(fields(&mut engine, 0).contains("@log:5 :1 insert:6"));
        ok(cmd_status_rm(&mut engine, "hp"));
        assert!(fields(&mut engine, 0).contains("@world :1 @read:6"), "the first hp only");
        ok(cmd_status_edit(&mut engine, "@log:1"));
        assert!(fields(&mut engine, 0).contains("@log:1 :1"));
        assert_eq!(engine.get_var("status_fields").map(|v| v.to_string_value()), Some(fields(&mut engine, 0)));

        ok(cmd_status_add(&mut engine, "-c a:3 b"));
        assert_eq!(fields(&mut engine, 0), "a:3 b");
        match cmd_status_add(&mut engine, "c") {
            TfCommandResult::Error(e) => assert_eq!(e, "STATUS_ADD: Only one variable width status field is allowed."),
            other => panic!("{:?}", other),
        }
        assert_eq!(fields(&mut engine, 0), "a:3 b", "an error changes nothing");
        ok(cmd_status_add(&mut engine, "-c a:2 b:3"));
        ok(cmd_status_add(&mut engine, "-x b:4 c:5"));
        assert_eq!(fields(&mut engine, 0), "a:2 b:3 :1 c:5", "-x skips each field that's there");
        ok(cmd_status_add(&mut engine, "-B -s0 first:2"));
        assert_eq!(fields(&mut engine, 0), "first:2 a:2 b:3 :1 c:5");

        for (row, expected) in [(":1 a:2 :3", ":3"), (":3 a:2 :1", ":3"), (":2 a:2 :2", ":2"), ("x:2 :1 @world", ":1 @world")] {
            ok(cmd_status_add(&mut engine, &format!("-c {}", row)));
            let target = if row.starts_with('x') { "x" } else { "a" };
            ok(cmd_status_rm(&mut engine, target));
            let want = if row.starts_with('x') { "@world" } else { expected };
            assert_eq!(fields(&mut engine, 0), want, "rm {} from {}", target, row);
        }
        match cmd_status_rm(&mut engine, "-r1 x") {
            TfCommandResult::Error(e) => assert_eq!(e, "STATUS_RM: x: no such field in row 1"),
            other => panic!("{:?}", other),
        }
        match cmd_status_add(&mut engine, "-Anosuch x:2") {
            TfCommandResult::Error(e) => assert_eq!(e, "STATUS_ADD: nosuch: no such field"),
            other => panic!("{:?}", other),
        }
        match cmd_status_edit(&mut engine, "nosuch:3") {
            TfCommandResult::Error(e) => assert_eq!(e, "STATUS_EDIT: nosuch:3: no such field"),
            other => panic!("{:?}", other),
        }
        ok(cmd_status_add(&mut engine, "-r1 bar:5"));
        ok(cmd_status_add(&mut engine, "-r1 bar:5"));
        assert_eq!(fields(&mut engine, 1), "bar:5 :1 bar:5");
    }

    /// /clock, /status_save, /status_restore and /status_defaults as tfstatus.tf has them.
    #[test]
    fn test_clock_save_restore_defaults() {
        let mut engine = TfEngine::new();
        let _ = cmd_status_add(&mut engine, "-c @world");
        let _ = cmd_clock(&mut engine, "");
        assert_eq!(fields(&mut engine, 0), "@world :1 @clock:5");
        let _ = cmd_clock(&mut engine, "%I:%M:%S");
        assert_eq!(fields(&mut engine, 0), "@world :1 @clock:8");
        let _ = cmd_clock(&mut engine, "off");
        assert_eq!(fields(&mut engine, 0), "@world");
        let _ = cmd_clock(&mut engine, "on");
        assert_eq!(fields(&mut engine, 0), "@world :1 @clock:8");
        let _ = cmd_status_save(&mut engine, "mine");
        let _ = cmd_status_defaults(&mut engine);
        assert_eq!(fields(&mut engine, 0), StatusLayout::tf_default().fields_text(0));
        assert_eq!(engine.get_var("clock_format").map(|v| v.to_string_value()).as_deref(), Some("%H:%M"));
        let _ = cmd_status_restore(&mut engine, "mine");
        assert_eq!(fields(&mut engine, 0), "@world :1 @clock:8");
        engine.begin_frame();
        let _ = cmd_status_restore(&mut engine, "nosave");
        let frame = engine.end_frame();
        assert!(matches!(&frame[..], [super::super::effects::TfEffect::Error { msg, .. }] if msg == "% No saved status \"nosave\"."), "{:?}", frame);
    }

    /// The default fields evaluate as TF's do with no world; a variable field shows the
    /// variable, a literal itself, and attributes wrap only the text.
    #[test]
    fn test_evaluate_fields() {
        let mut engine = TfEngine::new();
        // As the App mirrors it (sync_tf_world_info): Clay's input line inserts.
        engine.set_global("insert", TfValue::String("on".to_string()));
        let view = evaluate(&mut engine, None);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.pad, "_");
        let texts: Vec<&str> = view.rows[0].iter().map(|c| c.text.as_str()).filter(|t| !t.is_empty()).collect();
        assert_eq!(texts[0], "(no world)");
        assert_eq!(texts.len(), 2, "the world and the clock: {:?}", texts);
        assert!(view.rows[0][2].flex);

        let _ = cmd_status_add(&mut engine, "-c \"HP:\" hp:4:B :1 @world");
        engine.set_global("hp", TfValue::Integer(42));
        let view = evaluate(&mut engine, Some("Mud"));
        let row = &view.rows[0];
        assert_eq!(row[0].text, "HP:");
        assert_eq!(row[0].width, 3);
        assert_eq!(row[1].text, "\x1b[1m42\x1b[0m");
        assert!(row[3].flex);
        assert_eq!(row[3].text, "!Mud", "a world that isn't open is marked, as in TF");
        assert_eq!(engine.current_world, None, "the foreground is put back");
        assert_eq!(layout_row(row, 20, &view.pad, &view.attr), "HP:\x1b[1m42\x1b[0m___!Mud________");
    }

    /// layout_row draws as real tf does: fixed widths, the flexible field filling the
    /// rest, right-justification, the rest of a short row padded, a long one cut.
    #[test]
    fn test_layout_row_as_tf_draws() {
        let c = |text: &str, width: u16, flex: bool, right: bool| StatusCell { text: text.to_string(), width, flex, right };
        let row = [c("AB", 4, false, false), c("", 2, false, false), c("(no world)", 0, true, false), c("", 1, false, false), c("Z", 3, false, true)];
        assert_eq!(layout_row(&row, 60, ".", ""), format!("AB....(no world){}..Z", ".".repeat(41)));
        let row = [c("A", 2, false, false), c("LONGTEXT", 3, false, false), c("Z", 3, false, true)];
        assert_eq!(layout_row(&row, 20, ".", ""), "A.LON..Z............");
        let row = [c("A", 2, false, false), c("", 50, false, false), c("BCDEFGHIJKLMNOP", 20, false, false)];
        assert_eq!(layout_row(&row, 60, "_", ""), format!("A_{}BCDEFGHI", "_".repeat(50)));
        assert_eq!(layout_row(&[c("x", 1, false, false)], 3, "_", "\x1b[7m"), "\x1b[7mx__\x1b[0m");
        // A field cut short keeps its closing reset; wide characters count two columns.
        assert_eq!(layout_row(&[c("\x1b[1mabcdef\x1b[0m", 3, false, false)], 4, "_", ""), "\x1b[1mabc\x1b[0m_");
        assert_eq!(layout_row(&[c("日本語", 5, false, false)], 6, "_", ""), "日本__");
    }
}
