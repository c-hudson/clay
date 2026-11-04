//! Where a room goes on the map. The game's own coordinates when it sends them; otherwise
//! beside the room it was entered from, one step in the direction taken. A room already
//! on the map keeps its place: only its name, area name and exits are brought up to date.

use std::time::Instant;

use super::gmcp::RoomInfo;
use super::model::{Dir, Exit, MapGraph, Placed, Room, RoomId};

/// How the user got to a room: the room left, and the direction taken when known.
#[derive(Clone, Debug, Default)]
pub struct Arrival {
    pub from: Option<RoomId>,
    /// The movement command last sent (typed or walked), and when.
    pub sent: Option<(Dir, Instant)>,
}

/// A movement command sent this long ago still explains the next room.
const SENT_FRESH_SECS: u64 = 10;

/// How far from where a room should go Clay looks for a free cell.
const RING_MAX: i32 = 4;

/// Record a room the game described. Returns whether anything on the map changed.
pub fn record(graph: &mut MapGraph, info: &RoomInfo, arrival: &Arrival, now: Instant) -> bool {
    let from = arrival.from.as_ref().and_then(|f| graph.get(f)).cloned();
    // The direction taken: the exit of the room left that leads here, else the movement
    // just sent, else the reverse of this room's exit back.
    let dir = from
        .as_ref()
        .and_then(|f| f.exit_to(&info.id).and_then(|e| e.dir))
        .or_else(|| arrival.sent.filter(|(_, at)| now.saturating_duration_since(*at).as_secs() < SENT_FRESH_SECS).map(|(d, _)| d))
        .or_else(|| from.as_ref().and_then(|f| info.exits.iter().find(|e| e.to.as_deref() == Some(f.id.as_str())).and_then(|e| e.dir)).map(Dir::opposite));
    let mut changed = false;
    // The room left learns where that exit goes, when the game didn't say.
    if let (Some(f), Some(d)) = (&from, dir) {
        if f.id != info.id {
            if let Some(mut f) = graph.get(&f.id).cloned() {
                if let Some(e) = f.exits.iter_mut().find(|e| e.dir == Some(d) && e.to.is_none()) {
                    e.to = Some(info.id.clone());
                    graph.insert(f);
                    changed = true;
                }
            }
        }
    }
    // The y axis's direction, from a move north or south between two rooms the game placed.
    if let (Some(f), Some(d), Some((_, gy, _))) = (&from, dir, info.coords) {
        if let (Some((_, fy, _)), Some((_, dy, _))) = (f.given, d.delta()) {
            if dy != 0 && gy != fy {
                let sign = if (gy - fy).signum() == dy.signum() { 1 } else { -1 };
                changed |= vote_y(graph, sign);
            }
        }
    }
    if let Some(known) = graph.get(&info.id).cloned() {
        let exits = merge_exits(&known.exits, &info.exits);
        let updated = Room { name: info.name.clone(), area_name: info.area_name.clone(), env: info.env.clone(), exits, ..known.clone() };
        if updated != known {
            graph.insert(updated);
            return true;
        }
        return changed;
    }
    let (x, y, z, placed, given) = place(graph, info, from.as_ref(), dir);
    graph.insert(Room {
        id: info.id.clone(),
        name: info.name.clone(),
        area: info.area.clone(),
        area_name: info.area_name.clone(),
        env: info.env.clone(),
        x,
        y,
        z,
        placed,
        given,
        exits: info.exits.clone(),
    });
    true
}

/// The game's exits as it now lists them, keeping where Clay learned an exit leads when
/// the game still doesn't say.
fn merge_exits(old: &[Exit], new: &[Exit]) -> Vec<Exit> {
    new.iter()
        .map(|e| match (&e.to, old.iter().find(|o| o.cmd == e.cmd)) {
            (None, Some(o)) => Exit { to: o.to.clone(), ..e.clone() },
            _ => e.clone(),
        })
        .collect()
}

/// A position: x, y, z.
type Xyz = (i32, i32, i32);

/// A new room's cell: (x, y, z, how it was placed, the game's coordinates).
fn place(graph: &MapGraph, info: &RoomInfo, from: Option<&Room>, dir: Option<Dir>) -> (i32, i32, i32, Placed, Option<Xyz>) {
    let same_area = from.filter(|f| f.area == info.area);
    if let Some((gx, gy, gz)) = info.coords {
        // The game's level, else the level the move led to.
        let z = gz.unwrap_or_else(|| match (same_area, dir.and_then(Dir::delta)) {
            (Some(f), Some((_, _, dz))) => f.z + dz,
            (Some(f), None) => f.z,
            _ => 0,
        });
        let y = gy * graph.meta.y_sign;
        let (x, y) = free_near(graph, &info.area, (gx, y, z), None).unwrap_or((gx, y));
        return (x, y, z, Placed::Given, Some((gx, gy, z)));
    }
    if let (Some(f), Some(d)) = (same_area, dir) {
        if let Some((dx, dy, dz)) = d.delta() {
            let want = (f.x + dx, f.y + dy, f.z + dz);
            if let Some((x, y)) = free_near(graph, &info.area, want, Some((dx, dy))) {
                return (x, y, want.2, Placed::Inferred, None);
            }
        } else if let Some((x, y)) = free_near(graph, &info.area, (f.x + 1, f.y, f.z), None) {
            // In, Out, or an exit by a name: beside the room left.
            return (x, y, f.z, Placed::Inferred, None);
        }
    }
    // Nothing to stand beside: the area's first room at the origin, else apart from the
    // rest, to the right of them.
    let z = same_area.map_or(0, |f| f.z);
    match graph.bounds(&info.area, z) {
        None if !graph.has_area(&info.area) => (0, 0, 0, Placed::Island, None),
        None => (0, 0, z, Placed::Island, None),
        Some((_, min_y, max_x, _)) => {
            let (x, y) = free_near(graph, &info.area, (max_x + 2, min_y, z), None).unwrap_or((max_x + 2, min_y));
            (x, y, z, Placed::Island, None)
        }
    }
}

/// The cell for a room that wants `want`: that one when free; else further along the way
/// it came (two or three steps); else the nearest free cell in rings around it, in a
/// fixed order, so the same moves always give the same map.
fn free_near(graph: &MapGraph, area: &str, want: (i32, i32, i32), along: Option<(i32, i32)>) -> Option<(i32, i32)> {
    let (x, y, z) = want;
    let free = |x: i32, y: i32| graph.at(area, x, y, z).is_none();
    if free(x, y) {
        return Some((x, y));
    }
    if let Some((dx, dy)) = along.filter(|d| *d != (0, 0)) {
        for k in 1..=2 {
            let (cx, cy) = (x + dx * k, y + dy * k);
            if free(cx, cy) {
                return Some((cx, cy));
            }
        }
    }
    for r in 1..=RING_MAX {
        for (cx, cy) in ring(x, y, r) {
            if free(cx, cy) {
                return Some((cx, cy));
            }
        }
    }
    None
}

/// The cells at Chebyshev distance `r` from (x, y), clockwise from straight above.
fn ring(x: i32, y: i32, r: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for dx in -r..=r {
        out.push((x + dx, y - r));
    }
    for dy in -r + 1..=r {
        out.push((x + r, y + dy));
    }
    for dx in (-r..r).rev() {
        out.push((x + dx, y + r));
    }
    for dy in (-r + 1..r).rev() {
        out.push((x - r, y + dy));
    }
    out
}

/// A move's vote on which way the game's y grows. Three in a row against the map's
/// current way turn it, and every room the game placed is placed again.
fn vote_y(graph: &mut MapGraph, sign: i32) -> bool {
    if sign == graph.meta.y_sign {
        let changed = graph.meta.y_doubts != 0;
        graph.meta.y_doubts = 0;
        return changed;
    }
    graph.meta.y_doubts += 1;
    if graph.meta.y_doubts < 3 {
        return true;
    }
    graph.meta.y_sign = sign;
    graph.meta.y_doubts = 0;
    graph.moved_all = true;
    let given: Vec<Room> = graph.rooms.values().filter(|r| r.placed == Placed::Given).cloned().collect();
    for mut r in given {
        if let Some((_, gy, _)) = r.given {
            r.y = gy * sign;
            graph.insert(r);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::gmcp::RoomInfo;

    pub(crate) fn info(id: &str, area: &str, exits: &[(&str, Option<&str>)]) -> RoomInfo {
        RoomInfo {
            id: id.into(),
            name: format!("Room {id}"),
            area: area.into(),
            area_name: area.into(),
            env: String::new(),
            coords: None,
            exits: exits.iter().map(|(c, to)| Exit { cmd: c.to_string(), dir: Dir::parse(c), to: to.map(str::to_string) }).collect(),
        }
    }

    /// A room reported: its id, the direction taken to it, its exits (command, where to).
    type Step<'a> = (&'a str, Option<&'a str>, &'a [(&'a str, Option<&'a str>)]);

    fn walk(graph: &mut MapGraph, steps: &[Step]) {
        let now = Instant::now();
        let mut from: Option<RoomId> = None;
        for (id, sent, exits) in steps {
            let arrival = Arrival { from: from.clone(), sent: sent.and_then(Dir::parse).map(|d| (d, now)) };
            record(graph, &info(id, "Town", exits), &arrival, now);
            from = Some(id.to_string());
        }
    }

    fn pos(graph: &MapGraph, id: &str) -> (i32, i32, i32) {
        let r = graph.get(id).unwrap();
        (r.x, r.y, r.z)
    }

    #[test]
    fn rooms_go_one_step_the_way_they_were_entered() {
        let mut g = MapGraph::default();
        walk(&mut g, &[("a", None, &[("e", Some("b"))]), ("b", None, &[("e", None), ("w", Some("a"))]), ("c", Some("e"), &[("s", None)]), ("d", Some("s"), &[("u", None)]), ("e", Some("u"), &[])]);
        assert_eq!(pos(&g, "a"), (0, 0, 0));
        assert_eq!(pos(&g, "b"), (1, 0, 0), "by a's exit to it");
        assert_eq!(pos(&g, "c"), (2, 0, 0), "by the move sent");
        assert_eq!(pos(&g, "d"), (2, 1, 0));
        assert_eq!(pos(&g, "e"), (2, 1, 1), "up is a level");
        assert_eq!(g.get("b").unwrap().exits.iter().find(|e| e.cmd == "e").unwrap().to.as_deref(), Some("c"), "b learned where east goes");
    }

    #[test]
    fn a_taken_cell_pushes_the_room_further_then_around() {
        let mut g = MapGraph::default();
        // a, east to b, south to c, west to d - where a is: d goes further west, not on a.
        walk(&mut g, &[("a", None, &[]), ("b", Some("e"), &[]), ("c", Some("s"), &[]), ("d", Some("w"), &[]), ("x", Some("n"), &[])]);
        assert_eq!(pos(&g, "d"), (0, 1, 0));
        assert_eq!(pos(&g, "x"), (0, -1, 0), "a is north of d: one further");
        let again = {
            let mut g2 = MapGraph::default();
            walk(&mut g2, &[("a", None, &[]), ("b", Some("e"), &[]), ("c", Some("s"), &[]), ("d", Some("w"), &[]), ("x", Some("n"), &[])]);
            g2.rooms.values().map(|r| (r.id.clone(), r.x, r.y, r.z)).collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(again, g.rooms.values().map(|r| (r.id.clone(), r.x, r.y, r.z)).collect(), "the same moves, the same map");
    }

    #[test]
    fn a_known_room_keeps_its_place_and_takes_new_names() {
        let mut g = MapGraph::default();
        walk(&mut g, &[("a", None, &[("e", None)]), ("b", Some("e"), &[])]);
        let mut renamed = info("a", "Town", &[("e", None)]);
        renamed.name = "Town Square".into();
        assert!(record(&mut g, &renamed, &Arrival { from: Some("b".into()), sent: None }, Instant::now()));
        assert_eq!(pos(&g, "a"), (0, 0, 0));
        assert_eq!(g.get("a").unwrap().name, "Town Square");
        assert_eq!(g.get("a").unwrap().exits[0].to.as_deref(), Some("b"), "what it learned stays");
        assert!(!record(&mut g, &renamed, &Arrival::default(), Instant::now()), "nothing new: nothing changed");
    }

    #[test]
    fn a_new_area_starts_its_own_frame_and_a_lost_room_stands_apart() {
        let mut g = MapGraph::default();
        walk(&mut g, &[("a", None, &[]), ("b", Some("e"), &[])]);
        record(&mut g, &info("m", "Mines", &[]), &Arrival { from: Some("b".into()), sent: Some((Dir::Down, Instant::now())) }, Instant::now());
        assert_eq!(pos(&g, "m"), (0, 0, 0), "a new area's first room");
        record(&mut g, &info("t", "Town", &[]), &Arrival::default(), Instant::now());
        assert_eq!(pos(&g, "t"), (3, 0, 0), "a teleport: beside the area's rooms");
        assert_eq!(g.get("t").unwrap().placed, Placed::Island);
    }

    #[test]
    fn the_games_coordinates_and_which_way_its_y_grows() {
        let mut g = MapGraph::default();
        let now = Instant::now();
        let at = |id: &str, x: i32, y: i32| RoomInfo { coords: Some((x, y, Some(0))), ..info(id, "#1", &[("n", None), ("s", None)]) };
        record(&mut g, &at("a", 5, 5), &Arrival::default(), now);
        // This game's y grows north: three moves north that raised y turn the axis.
        let mut from = "a".to_string();
        for (i, id) in ["b", "c", "d"].iter().enumerate() {
            record(&mut g, &at(id, 5, 6 + i as i32), &Arrival { from: Some(from.clone()), sent: Some((Dir::N, now)) }, now);
            from = id.to_string();
        }
        assert_eq!(g.meta.y_sign, -1);
        assert_eq!((pos(&g, "a").1, pos(&g, "d").1), (-5, -8), "north is up on screen");
        assert!(g.moved_all, "whoever keeps a copy takes every room again");
    }
}
