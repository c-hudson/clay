//! A shell world's screen as rows of ANSI text.
//!
//! Every client already turns a line of ANSI text into styled cells (the console's
//! terminal does it natively, app.js has `parseAnsi`, the native GUI `style_line`), so
//! the screen crosses the wire as exactly that: one string per row, SGR codes and all,
//! redrawn in place. `render_rows` reads the emulator; `ShellScreen` keeps the last
//! rendering, says which rows changed, and builds the `ShellScreen` frames clients get
//! (`delta_msg`, `full_msg`) - and applies them, for the clients written in Rust
//! (`apply_msg`: the SSH remote console's mirror and the native GUI).
//!
//! Activity (`Applied::gained_printable`) is read off the same rendering: a row counts
//! only when a cell gained a non-space character it didn't have. A cursor move, a colour
//! change or a clear is not activity; neither is `top` rewriting the same digits.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};

use crate::websocket::{ShellCursorMsg, ShellModeMsg, ShellScreenMsg, WsMessage};

/// The whole visible screen, as `render_rows` reads it off a `Term`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowsSnapshot {
    pub cols: u16,
    pub rows: u16,
    /// One ANSI string per row, trailing blank cells trimmed.
    pub lines: Vec<String>,
    /// The characters of each row, one per cell (a wide character once), for the
    /// activity rule.
    pub plain: Vec<Vec<char>>,
    pub cursor: ShellCursorMsg,
    pub mode: ShellModeMsg,
}

/// A cell's appearance, as SGR codes say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Attrs {
    fg: Col,
    bg: Col,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
    strike: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Col {
    #[default]
    Default,
    /// One of the 16: 30-37 / 90-97.
    Named(u8),
    /// 38;5;n.
    Indexed(u8),
    /// 38;2;r;g;b.
    Rgb(Rgb),
}

impl Attrs {
    fn of(cell: &Cell, colors: &Colors) -> Attrs {
        let flags = cell.flags;
        let mut a = Attrs {
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            inverse: flags.contains(Flags::INVERSE),
            strike: flags.contains(Flags::STRIKEOUT),
            ..Attrs::default()
        };
        let (fg, fg_dim) = col(cell.fg, colors);
        let (bg, _) = col(cell.bg, colors);
        a.fg = fg;
        a.bg = bg;
        a.dim |= fg_dim;
        a
    }

    /// A blank cell with these attributes still shows something.
    fn significant(&self) -> bool {
        self.inverse || self.underline || self.strike || self.bg != Col::Default
    }

    /// The SGR sequence that makes a terminal draw like this, from a reset.
    fn write_sgr(&self, out: &mut String) {
        use std::fmt::Write;
        out.push_str("\x1b[0");
        if self.bold {
            out.push_str(";1");
        }
        if self.dim {
            out.push_str(";2");
        }
        if self.italic {
            out.push_str(";3");
        }
        if self.underline {
            out.push_str(";4");
        }
        if self.inverse {
            out.push_str(";7");
        }
        if self.strike {
            out.push_str(";9");
        }
        match self.fg {
            Col::Default => {}
            Col::Named(n) if n < 8 => { let _ = write!(out, ";{}", 30 + n); }
            Col::Named(n) => { let _ = write!(out, ";{}", 90 + (n - 8)); }
            Col::Indexed(n) => { let _ = write!(out, ";38;5;{n}"); }
            Col::Rgb(c) => { let _ = write!(out, ";38;2;{};{};{}", c.r, c.g, c.b); }
        }
        match self.bg {
            Col::Default => {}
            Col::Named(n) if n < 8 => { let _ = write!(out, ";{}", 40 + n); }
            Col::Named(n) => { let _ = write!(out, ";{}", 100 + (n - 8)); }
            Col::Indexed(n) => { let _ = write!(out, ";48;5;{n}"); }
            Col::Rgb(c) => { let _ = write!(out, ";48;2;{};{};{}", c.r, c.g, c.b); }
        }
        out.push('m');
    }
}

/// A cell colour as SGR, honouring a palette entry the program changed (OSC 4). The
/// bool: alacritty's "dim" named colours, which SGR spells as dim + the colour.
fn col(color: Color, colors: &Colors) -> (Col, bool) {
    match color {
        Color::Spec(rgb) => (Col::Rgb(rgb), false),
        Color::Indexed(i) => (colors[i as usize].map_or(Col::Indexed(i), Col::Rgb), false),
        Color::Named(n) => {
            if let Some(rgb) = colors[n] {
                return (Col::Rgb(rgb), false);
            }
            use NamedColor::*;
            let c = match n {
                Black => Col::Named(0),
                Red => Col::Named(1),
                Green => Col::Named(2),
                Yellow => Col::Named(3),
                Blue => Col::Named(4),
                Magenta => Col::Named(5),
                Cyan => Col::Named(6),
                White => Col::Named(7),
                BrightBlack => Col::Named(8),
                BrightRed => Col::Named(9),
                BrightGreen => Col::Named(10),
                BrightYellow => Col::Named(11),
                BrightBlue => Col::Named(12),
                BrightMagenta => Col::Named(13),
                BrightCyan => Col::Named(14),
                BrightWhite => Col::Named(15),
                DimBlack => return (Col::Named(0), true),
                DimRed => return (Col::Named(1), true),
                DimGreen => return (Col::Named(2), true),
                DimYellow => return (Col::Named(3), true),
                DimBlue => return (Col::Named(4), true),
                DimMagenta => return (Col::Named(5), true),
                DimCyan => return (Col::Named(6), true),
                DimWhite => return (Col::Named(7), true),
                DimForeground => return (Col::Default, true),
                Foreground | Background | Cursor | BrightForeground => Col::Default,
            };
            (c, false)
        }
    }
}

/// The emulator's modes a client (or the key encoder) needs to know.
pub fn mode_msg(mode: TermMode) -> ShellModeMsg {
    ShellModeMsg {
        mouse_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
        mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
        mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
        sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
        utf8_mouse: mode.contains(TermMode::UTF8_MOUSE),
        alt_screen: mode.contains(TermMode::ALT_SCREEN),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        app_keypad: mode.contains(TermMode::APP_KEYPAD),
        alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
    }
}

/// Read the visible screen off the emulator.
pub fn render_rows<T: EventListener>(term: &Term<T>) -> RowsSnapshot {
    let cols = term.columns();
    let rows = term.screen_lines();
    let colors = *term.colors();
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let mut lines: Vec<String> = vec![String::new(); rows];
    let mut plain: Vec<Vec<char>> = vec![vec![' '; cols]; rows];
    // Byte length of each row up to its last cell that shows something.
    let mut keep = vec![0usize; rows];
    let mut cur_row = usize::MAX;
    let mut current = Attrs::default();
    for indexed in content.display_iter {
        let y = (indexed.point.line.0 + offset) as usize;
        let x = indexed.point.column.0;
        if y >= rows || x >= cols {
            continue;
        }
        let cell = indexed.cell;
        if y != cur_row {
            cur_row = y;
            current = Attrs::default();
        }
        // A wide character's second half carries no glyph of its own.
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let line = &mut lines[y];
        let attrs = Attrs::of(cell, &colors);
        if attrs != current {
            attrs.write_sgr(line);
            current = attrs;
        }
        let ch = if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
        line.push(ch);
        if let Some(zerowidth) = cell.zerowidth() {
            for &z in zerowidth {
                line.push(z);
            }
        }
        plain[y][x] = ch;
        // Trailing blanks (a space with nothing else to show) are trimmed; anything that
        // draws, or a non-space, extends the kept length to here.
        if ch != ' ' || attrs.significant() {
            keep[y] = line.len();
        }
    }
    for (line, &k) in lines.iter_mut().zip(keep.iter()) {
        line.truncate(k);
        if !line.is_empty() {
            line.push_str("\x1b[0m");
        }
    }

    let rc = term.renderable_content();
    let point = rc.cursor.point;
    let cursor = ShellCursorMsg {
        col: point.column.0 as u16,
        row: (point.line.0 + offset) as u16,
        shape: shape_code(rc.cursor.shape),
        visible: rc.cursor.shape != CursorShape::Hidden,
        blink: term.cursor_style().blinking,
    };

    RowsSnapshot { cols: cols as u16, rows: rows as u16, lines, plain, cursor, mode: mode_msg(*term.mode()) }
}

fn shape_code(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Beam => 2,
        CursorShape::HollowBlock => 3,
        CursorShape::Hidden => 0,
    }
}

/// The server's cache of a world's screen: the last rendering, plus a frame counter so a
/// client can tell a delta applies to what it holds.
#[derive(Debug, Clone)]
pub struct ShellScreen {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<String>,
    plain: Vec<Vec<char>>,
    pub cursor: ShellCursorMsg,
    pub mode: ShellModeMsg,
    pub title: Option<String>,
    pub bell: bool,
    /// The emulator is still running. Once false the screen is frozen (the exit note is
    /// the last thing drawn).
    pub live: bool,
    /// The frame clients currently hold. Advanced only when a delta is broadcast
    /// (`delta_msg`), so every viewer stays in lockstep: a delta names this as its
    /// `base_frame` and carries `frame + 1`.
    pub frame: u64,
    /// The rows that changed since the last delta.
    dirty: Vec<bool>,
    /// Something a viewer would see changed since the last delta (rows, cursor, mode,
    /// title or liveness) - a delta is due.
    changed: bool,
    /// No frame has been broadcast for this screen yet, so the first one is sent full (not
    /// a delta a viewing client with no screen would have to reject and re-request).
    fresh: bool,
    /// A resize just happened: suppress the reflow as activity for one rendering.
    resized: bool,
    /// Until this instant, what appears on the screen is not activity: the repaint a
    /// program does after a `/reload` re-attaches it (the size nudge, the injected note)
    /// is Clay's doing, not news from the program, and used to light the world's activity
    /// indicator on every reload.
    quiet_until: Option<std::time::Instant>,
    /// Client side: a delta didn't build on the frame held and the whole screen has been
    /// asked for. Until it arrives every delta is stale, so none of them asks again.
    awaiting_full: bool,
}

/// What applying a rendering did.
pub struct Applied {
    /// A row gained a printable character: activity, for a world nobody is viewing.
    pub gained_printable: bool,
    /// Something a viewer would see changed (so the console repaints, a delta is due).
    pub changed: bool,
}

impl ShellScreen {
    /// A fresh, live, blank screen.
    pub fn new(cols: u16, rows: u16) -> ShellScreen {
        ShellScreen {
            cols,
            rows,
            lines: vec![String::new(); rows as usize],
            plain: vec![vec![' '; cols as usize]; rows as usize],
            cursor: ShellCursorMsg::default(),
            mode: ShellModeMsg::default(),
            title: None,
            bell: false,
            live: true,
            frame: 1,
            dirty: vec![true; rows as usize],
            changed: true,
            fresh: true,
            resized: false,
            quiet_until: None,
            awaiting_full: false,
        }
    }

    /// Nothing that appears before `until` counts as activity (see `quiet_until`).
    pub fn ignore_activity_until(&mut self, until: std::time::Instant) {
        self.quiet_until = Some(until);
    }

    /// The next rendering of the emulator. Returns what changed (activity, repaint).
    pub fn apply(&mut self, snap: RowsSnapshot) -> Applied {
        let resized = snap.cols != self.cols || snap.rows != self.rows;
        let quiet = match self.quiet_until {
            Some(until) if std::time::Instant::now() < until => true,
            Some(_) => {
                self.quiet_until = None;
                false
            }
            None => false,
        };
        let mut gained_printable = false;
        let mut changed = resized;
        if resized {
            self.cols = snap.cols;
            self.rows = snap.rows;
            self.dirty = vec![true; snap.rows as usize];
        } else if self.dirty.len() != snap.lines.len() {
            self.dirty = vec![false; snap.lines.len()];
        }
        for y in 0..snap.lines.len() {
            if self.lines.get(y) != Some(&snap.lines[y]) {
                changed = true;
                if let Some(d) = self.dirty.get_mut(y) {
                    *d = true;
                }
            }
            // Activity: a cell gained a non-space it didn't have (reflow and a reload's
            // repaint excepted).
            if !resized && !self.resized && !quiet {
                let old = self.plain.get(y);
                for (x, &ch) in snap.plain[y].iter().enumerate() {
                    if ch != ' ' && old.and_then(|row| row.get(x)).copied().unwrap_or(' ') != ch {
                        gained_printable = true;
                        break;
                    }
                }
            }
        }
        if self.cursor != snap.cursor || self.mode != snap.mode {
            changed = true;
        }
        self.lines = snap.lines;
        self.plain = snap.plain;
        self.cursor = snap.cursor;
        self.mode = snap.mode;
        self.resized = resized;
        self.changed |= changed;
        Applied { gained_printable, changed }
    }

    /// Note the program set or cleared the title.
    pub fn set_title(&mut self, title: Option<String>) {
        if self.title != title {
            self.title = title;
            self.changed = true;
        }
    }

    /// The session ended: freeze the screen.
    pub fn set_exited(&mut self) {
        self.live = false;
        self.changed = true;
    }

    /// A viewer would see a change that hasn't been broadcast yet.
    pub fn has_changes(&self) -> bool {
        self.changed
    }

    /// A full frame: every row, no base, at the frame clients hold now (not advanced).
    /// For a client that just switched in, reconnected, or fell behind.
    pub fn full_msg(&self, world_index: usize) -> WsMessage {
        let lines = self.lines.iter().cloned().enumerate().map(|(i, s)| (i as u16, s)).collect();
        self.msg(world_index, None, lines)
    }

    /// A delta of the rows changed since the last delta, built on the frame clients hold,
    /// then advancing that frame. Clears the dirty/changed state. Broadcast to every
    /// viewer at once, so they stay in lockstep.
    pub fn delta_msg(&mut self, world_index: usize) -> WsMessage {
        // The very first frame for a screen is sent full: a client already viewing the
        // world when its shell connects has no screen yet and would otherwise have to
        // reject a delta and re-request.
        if std::mem::take(&mut self.fresh) {
            self.changed = false;
            for d in &mut self.dirty {
                *d = false;
            }
            return self.full_msg(world_index);
        }
        let base = self.frame;
        self.frame += 1;
        let lines: Vec<(u16, String)> = self
            .dirty
            .iter()
            .enumerate()
            .filter(|(_, &d)| d)
            .map(|(i, _)| (i as u16, self.lines.get(i).cloned().unwrap_or_default()))
            .collect();
        for d in &mut self.dirty {
            *d = false;
        }
        self.changed = false;
        self.msg(world_index, Some(base), lines)
    }

    fn msg(&self, world_index: usize, base_frame: Option<u64>, lines: Vec<(u16, String)>) -> WsMessage {
        WsMessage::ShellScreen {
            world_index,
            frame: self.frame,
            base_frame,
            cols: self.cols,
            rows: self.rows,
            lines,
            cursor: self.cursor,
            mode: self.mode,
            title: self.title.clone(),
            bell: self.bell,
            live: self.live,
        }
    }


    /// For `WorldStateMsg.shell`: the screen as it stands.
    pub fn to_state_msg(&self) -> ShellScreenMsg {
        ShellScreenMsg {
            frame: self.frame,
            cols: self.cols,
            rows: self.rows,
            lines: self.lines.clone(),
            cursor: self.cursor,
            mode: self.mode,
            title: self.title.clone(),
            live: self.live,
        }
    }

    /// Build a client-side screen from a `WorldStateMsg.shell` (a full snapshot).
    pub fn from_state_msg(msg: &ShellScreenMsg) -> ShellScreen {
        let rows = msg.rows as usize;
        let mut lines = msg.lines.clone();
        lines.resize(rows, String::new());
        ShellScreen {
            cols: msg.cols,
            rows: msg.rows,
            lines,
            plain: Vec::new(),
            cursor: msg.cursor,
            mode: msg.mode,
            title: msg.title.clone(),
            bell: false,
            live: msg.live,
            frame: msg.frame,
            dirty: Vec::new(),
            changed: false,
            fresh: false,
            resized: false,
            quiet_until: None,
            awaiting_full: false,
        }
    }

    /// Apply a `ShellScreen` frame from the server (the SSH mirror and the native GUI).
    /// Returns false when the delta doesn't build on the frame held, so the caller knows
    /// to ask for a full one - once: the deltas still in flight until that full frame
    /// arrives can't build on it either, and each used to cost another request and
    /// another full frame (a resize while a program was writing sent a dozen). They are
    /// dropped (true) until the full frame lands; a delta rejected after it asks again.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_msg(
        &mut self,
        frame: u64,
        base_frame: Option<u64>,
        cols: u16,
        rows: u16,
        lines: &[(u16, String)],
        cursor: ShellCursorMsg,
        mode: ShellModeMsg,
        title: Option<String>,
        live: bool,
    ) -> bool {
        match base_frame {
            None => {
                self.awaiting_full = false;
                self.lines = vec![String::new(); rows as usize];
                for (i, s) in lines {
                    if let Some(row) = self.lines.get_mut(*i as usize) {
                        *row = s.clone();
                    }
                }
            }
            Some(base) => {
                if base != self.frame || cols != self.cols || rows != self.rows {
                    let already_asked = std::mem::replace(&mut self.awaiting_full, true);
                    return already_asked;
                }
                for (i, s) in lines {
                    if let Some(row) = self.lines.get_mut(*i as usize) {
                        *row = s.clone();
                    }
                }
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.lines.resize(rows as usize, String::new());
        self.cursor = cursor;
        self.mode = mode;
        self.title = title;
        self.live = live;
        self.frame = frame;
        true
    }
}
