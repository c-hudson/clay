//! The map drawn in text, for the console's map pane (local and SSH alike): each room a
//! `[ ]` four columns and two rows from its neighbours, the links between them in the
//! cells between, the user `[@]`, the destination `[X]`, the route in yellow.

use std::collections::HashSet;

use super::wire::{MapRoomMsg, MapStateMsg};

const CELL_W: i32 = 4;
const CELL_H: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    Map,
    Here,
    Target,
    Route,
    Dim,
    Elsewhere,
}

impl Ink {
    fn sgr(self) -> &'static str {
        match self {
            Ink::Map => "\x1b[38;5;110m",
            Ink::Here => "\x1b[1;38;5;209m",
            Ink::Target => "\x1b[1;38;5;221m",
            Ink::Route => "\x1b[38;5;221m",
            Ink::Dim => "\x1b[38;5;244m",
            Ink::Elsewhere => "\x1b[38;5;80m",
        }
    }
}

/// What to draw: the rooms of one area's level, centred on a room or a cell.
pub struct View<'a> {
    pub rooms: &'a [MapRoomMsg],
    pub here: Option<&'a str>,
    pub target: Option<&'a str>,
    /// The rooms the route passes through, the destination included.
    pub route: &'a [String],
    pub center: (i32, i32),
}

/// The map as `rows` lines of ANSI text, each `cols` columns wide (blank where there's
/// nothing). `empty` is shown, centred, when there are no rooms at all.
pub fn render(view: &View, cols: usize, rows: usize, empty: &str) -> Vec<String> {
    let mut grid = vec![vec![(' ', Ink::Map); cols]; rows];
    if view.rooms.is_empty() {
        if rows > 0 {
            let text: String = empty.chars().take(cols).collect();
            let pad = cols.saturating_sub(text.chars().count()) / 2;
            let mut line = " ".repeat(pad);
            line.push_str(&text);
            let mut out = vec![String::new(); rows];
            out[rows / 2] = format!("{}{}\x1b[0m", Ink::Dim.sgr(), line);
            return out;
        }
        return Vec::new();
    }
    let on_route: HashSet<&str> = view.route.iter().map(String::as_str).chain(view.here).collect();
    let (mid_c, mid_r) = (cols as i32 / 2, rows as i32 / 2);
    let at = |x: i32, y: i32| (mid_c + (x - view.center.0) * CELL_W - 1, mid_r + (y - view.center.1) * CELL_H);
    let put = |grid: &mut Vec<Vec<(char, Ink)>>, c: i32, r: i32, ch: char, ink: Ink| {
        if c < 0 || r < 0 || c as usize >= cols || r as usize >= rows {
            return;
        }
        let cell = &mut grid[r as usize][c as usize];
        // Two links in one cell (diagonals crossing): an X.
        cell.0 = if cell.0 != ' ' && cell.0 != ch && "-|/\\".contains(cell.0) && "-|/\\".contains(ch) { 'X' } else { ch };
        if cell.1 == Ink::Map || ink != Ink::Map {
            cell.1 = ink;
        }
    };
    for room in view.rooms {
        let (c, r) = at(room.x, room.y);
        for e in &room.exits {
            let Some((dc, dr, ch)) = e.dir.as_deref().and_then(link) else { continue };
            let (ch, ink) = match e.kind {
                3 => ('·', Ink::Dim),
                2 => ('>', Ink::Elsewhere),
                _ if on_route.contains(room.id.as_str()) && e.to.as_deref().is_some_and(|t| on_route.contains(t)) => (ch, Ink::Route),
                _ => (ch, Ink::Map),
            };
            put(&mut grid, c + dc, r + dr, ch, ink);
        }
    }
    for room in view.rooms {
        let (c, r) = at(room.x, room.y);
        let (up, down) = (room.exits.iter().any(|e| e.dir.as_deref() == Some("u")), room.exits.iter().any(|e| e.dir.as_deref() == Some("d")));
        let (mark, ink) = if Some(room.id.as_str()) == view.here {
            ('@', Ink::Here)
        } else if Some(room.id.as_str()) == view.target {
            ('X', Ink::Target)
        } else {
            let mark = match (up, down) {
                (true, true) => '*',
                (true, false) => '^',
                (false, true) => 'v',
                _ => ' ',
            };
            (mark, if on_route.contains(room.id.as_str()) { Ink::Route } else { Ink::Map })
        };
        put(&mut grid, c, r, '[', ink);
        put(&mut grid, c + 1, r, mark, ink);
        put(&mut grid, c + 2, r, ']', ink);
    }
    grid.into_iter()
        .map(|row| {
            let mut s = String::new();
            let mut ink: Option<Ink> = None;
            for (ch, k) in row {
                if ch != ' ' && ink != Some(k) {
                    s.push_str(k.sgr());
                    ink = Some(k);
                }
                s.push(ch);
            }
            let trimmed = s.trim_end().to_string();
            if ink.is_some() { format!("{trimmed}\x1b[0m") } else { trimmed }
        })
        .collect()
}

/// Where a link in direction `dir` is drawn, from the room's `[`: (column, row, glyph).
fn link(dir: &str) -> Option<(i32, i32, char)> {
    Some(match dir {
        "e" => (3, 0, '-'),
        "w" => (-1, 0, '-'),
        "n" => (1, -1, '|'),
        "s" => (1, 1, '|'),
        "ne" => (3, -1, '/'),
        "sw" => (-1, 1, '/'),
        "nw" => (-1, -1, '\\'),
        "se" => (3, 1, '\\'),
        _ => return None,
    })
}

/// The map pane's bar: the area, level, size, and the walk or why it last stopped.
pub fn bar_text(world: &str, state: Option<&MapStateMsg>) -> String {
    let Some(s) = state.filter(|s| s.rooms_total > 0) else { return format!("Map · {world} · no map yet") };
    let area = if s.area_name.is_empty() { s.area.as_str() } else { s.area_name.as_str() };
    let mut out = format!("Map · {area} · level {} · {} rooms", s.level, s.rooms_total);
    if let Some(w) = &s.walk {
        out.push_str(&format!(" · Walking {}/{} to {} (Esc stops)", w.next + 1, w.steps.len(), w.to_name));
    } else if let Some(stop) = &s.last_stop {
        out.push_str(" · ");
        out.push_str(stop);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::path::tests_support::rome;
    use crate::map::wire::snapshot_area;

    fn plain(lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| crate::util::strip_ansi_codes(l)).collect()
    }

    #[test]
    fn rooms_links_you_and_the_destination() {
        let g = rome();
        let (rooms, _) = snapshot_area(&g, "Forum", 0, (1, 2));
        let route: Vec<String> = ["G", "H", "K", "L", "M"].iter().map(|s| s.to_string()).collect();
        let view = View { rooms: &rooms, here: Some("F"), target: Some("M"), route: &route, center: (2, 2) };
        let text = plain(&render(&view, 24, 11, ""));
        assert_eq!(text, [
            "",
            "   [ ]-[ ]",
            "        |",
            "       [ ]",
            "        | \\",
            "   [ ]-[@]-[ ]-[ ]",
            "        |       |",
            "       [ ]     [ ]",
            "                |",
            "               [ ]-[X]",
            "",
        ]);
        let coloured = render(&view, 24, 11, "");
        assert!(coloured[5].contains("\x1b[1;38;5;209m[@]"), "you, in orange");
        assert!(coloured[9].contains("\x1b[1;38;5;221m[X]"), "the destination, in bold yellow");
    }

    #[test]
    fn no_rooms_says_so_and_the_bar_says_where() {
        let text = plain(&render(&View { rooms: &[], here: None, target: None, route: &[], center: (0, 0) }, 20, 3, "No map yet"));
        assert_eq!(text, ["", "     No map yet", ""]);
        assert_eq!(bar_text("Rome", None), "Map · Rome · no map yet");
        let state = MapStateMsg { area: "Forum".into(), area_name: "Forum Romanum".into(), level: 0, rooms_total: 12, ..Default::default() };
        assert_eq!(bar_text("Rome", Some(&state)), "Map · Forum Romanum · level 0 · 12 rooms");
    }
}
