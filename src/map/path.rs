//! Routes through the map, and finding a room by what the user typed.

use std::collections::{HashMap, VecDeque};

use super::model::{MapGraph, RoomId};

/// One step of a route: the command to send, and the room it should lead to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub cmd: String,
    pub to: RoomId,
}

/// The longest search a route may make (rooms visited).
const VISIT_CAP: usize = 200_000;

/// The fewest steps from one room to another, over exits whose rooms are known. `None`
/// when there's no way there; an empty route when already there.
pub fn route(graph: &MapGraph, from: &str, to: &str) -> Option<Vec<Step>> {
    if from == to {
        return Some(Vec::new());
    }
    graph.get(from)?;
    graph.get(to)?;
    let mut prev: HashMap<&str, (&str, &str)> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::from([from]);
    let mut seen = 0usize;
    while let Some(cur) = queue.pop_front() {
        seen += 1;
        if cur == to || seen > VISIT_CAP {
            break;
        }
        let Some(room) = graph.get(cur) else { continue };
        for exit in &room.exits {
            let Some(next) = exit.to.as_deref() else { continue };
            if next == from || prev.contains_key(next) || graph.get(next).is_none() {
                continue;
            }
            prev.insert(next, (cur, exit.cmd.as_str()));
            queue.push_back(next);
        }
    }
    let mut steps = Vec::new();
    let mut at = to;
    while at != from {
        let (back, cmd) = prev.get(at)?;
        steps.push(Step { cmd: cmd.to_string(), to: at.to_string() });
        at = back;
    }
    steps.reverse();
    Some(steps)
}

/// What a typed destination named: one room, or several (the best ones, by id and name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    One(RoomId),
    Many(Vec<RoomId>),
    None,
}

/// Find the room the user means: `#id` or an exact id first; then by name, whole name,
/// then its start, then anywhere in it, any case. Among rooms with the same best name,
/// the nearest to `near` (fewest steps) wins.
pub fn resolve(graph: &MapGraph, query: &str, near: Option<&str>) -> Target {
    let q = query.trim();
    if q.is_empty() {
        return Target::None;
    }
    let id = q.strip_prefix('#').unwrap_or(q);
    if graph.get(id).is_some() {
        return Target::One(id.to_string());
    }
    let found = search(graph, q, near, usize::MAX);
    match found.as_slice() {
        [] => Target::None,
        [one] => Target::One(one.clone()),
        [first, rest @ ..] => {
            let name = |id: &RoomId| graph.get(id).map(|r| r.name.to_lowercase()).unwrap_or_default();
            let tier = |id: &RoomId| match_tier(&name(id), &q.to_lowercase());
            // The best tier holds one name: the nearest of the rooms that carry it.
            let best: Vec<&RoomId> = std::iter::once(first).chain(rest).filter(|r| tier(r) == tier(first)).collect();
            if best.iter().all(|r| name(r) == name(first)) && near.is_some() {
                Target::One(first.clone())
            } else {
                Target::Many(found.into_iter().take(10).collect())
            }
        }
    }
}

/// How well a name matches: 0 the whole name, 1 its start, 2 anywhere, None not at all.
fn match_tier(name: &str, q: &str) -> Option<u8> {
    if name == q {
        Some(0)
    } else if name.starts_with(q) {
        Some(1)
    } else if name.contains(q) {
        Some(2)
    } else {
        None
    }
}

/// Rooms whose name matches, best first: by how well, then the same area as `near`
/// first, then the nearest, then by name and id. At most `limit`.
pub fn search(graph: &MapGraph, query: &str, near: Option<&str>, limit: usize) -> Vec<RoomId> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let here = near.and_then(|n| graph.get(n));
    let dist = here.map(|h| distances(graph, &h.id)).unwrap_or_default();
    let mut hits: Vec<(u8, bool, usize, String, RoomId)> = graph
        .rooms
        .values()
        .filter_map(|r| {
            let tier = match_tier(&r.name.to_lowercase(), &q)?;
            let other_area = here.is_some_and(|h| h.area != r.area);
            Some((tier, other_area, dist.get(r.id.as_str()).copied().unwrap_or(usize::MAX), r.name.to_lowercase(), r.id.clone()))
        })
        .collect();
    hits.sort();
    hits.into_iter().take(limit).map(|h| h.4).collect()
}

/// Steps from one room to every room it reaches.
fn distances<'a>(graph: &'a MapGraph, from: &'a str) -> HashMap<&'a str, usize> {
    let mut dist = HashMap::from([(from, 0usize)]);
    let mut queue = VecDeque::from([from]);
    while let Some(cur) = queue.pop_front() {
        if dist.len() > VISIT_CAP {
            break;
        }
        let d = dist[cur];
        let Some(room) = graph.get(cur) else { continue };
        for next in room.exits.iter().filter_map(|e| e.to.as_deref()) {
            if graph.get(next).is_some() && !dist.contains_key(next) {
                dist.insert(next, d + 1);
                queue.push_back(next);
            }
        }
    }
    dist
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::map::model::{Dir, Exit, Placed, Room};

    /// A room: id, name, x, y, exits (command, to).
    type RoomDef<'a> = (&'a str, &'a str, i32, i32, &'a [(&'a str, &'a str)]);

    /// The mockup's Rome: The Forum (F) to the Palatine Steps (M) is e, e, s, s, e.
    pub(crate) fn rome() -> MapGraph {
        let mut g = MapGraph::default();
        let rooms: &[RoomDef] = &[
            ("A", "Temple of Saturn", 0, 0, &[("e", "B")]),
            ("B", "Clivus Capitolinus", 1, 0, &[("w", "A"), ("s", "D")]),
            ("D", "Rostra", 1, 1, &[("n", "B"), ("s", "F"), ("se", "G")]),
            ("E", "Basilica Julia", 0, 2, &[("e", "F")]),
            ("F", "The Forum", 1, 2, &[("n", "D"), ("w", "E"), ("e", "G"), ("s", "J")]),
            ("G", "Via Sacra", 2, 2, &[("w", "F"), ("nw", "D"), ("e", "H")]),
            ("H", "Via Sacra, East", 3, 2, &[("w", "G"), ("s", "K")]),
            ("J", "Curia Julia", 1, 3, &[("n", "F")]),
            ("K", "Temple of Vesta", 3, 3, &[("n", "H"), ("s", "L")]),
            ("L", "House of the Vestals", 3, 4, &[("n", "K"), ("e", "M")]),
            ("M", "Palatine Steps", 4, 4, &[("w", "L"), ("u", "")]),
            ("V", "Temple of Vesta", 9, 9, &[]),
        ];
        for (id, name, x, y, exits) in rooms {
            g.insert(Room {
                id: id.to_string(),
                name: name.to_string(),
                area: "Forum".into(),
                area_name: "Forum Romanum".into(),
                env: String::new(),
                x: *x,
                y: *y,
                z: 0,
                placed: Placed::Inferred,
                given: None,
                exits: exits.iter().map(|(c, to)| Exit { cmd: c.to_string(), dir: Dir::parse(c), to: (!to.is_empty()).then(|| to.to_string()) }).collect(),
            });
        }
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::rome;

    fn cmds(r: &[Step]) -> Vec<&str> {
        r.iter().map(|s| s.cmd.as_str()).collect()
    }

    #[test]
    fn the_fewest_steps() {
        let g = rome();
        assert_eq!(cmds(&route(&g, "F", "M").unwrap()), ["e", "e", "s", "s", "e"]);
        assert_eq!(route(&g, "F", "M").unwrap().last().unwrap().to, "M");
        assert_eq!(cmds(&route(&g, "A", "G").unwrap()), ["e", "s", "se"]);
        assert_eq!(route(&g, "F", "F"), Some(Vec::new()));
        assert_eq!(route(&g, "F", "V"), None, "no way there");
        assert_eq!(route(&g, "F", "nowhere"), None);
    }

    #[test]
    fn finding_a_room_by_id_or_name() {
        let g = rome();
        assert_eq!(resolve(&g, "#M", None), Target::One("M".into()));
        assert_eq!(resolve(&g, "palatine", Some("F")), Target::One("M".into()), "the start of a name");
        assert_eq!(resolve(&g, "the forum", None), Target::One("F".into()));
        assert_eq!(resolve(&g, "temple of vesta", Some("F")), Target::One("K".into()), "two of that name: the nearest");
        assert!(matches!(resolve(&g, "temple", Some("F")), Target::Many(v) if v.len() == 3), "different names: the user picks");
        assert_eq!(resolve(&g, "atlantis", None), Target::None);
        assert_eq!(search(&g, "via", Some("F"), 5), ["G", "H"]);
    }
}
