//! Reading a room out of GMCP. Games send the room you're in as `Room.Info` (IRE's and
//! most others') or `room.info` (Aardwolf's), each its own shape; object keys are matched
//! in any case. A failed move is `Room.WrongDir`.

use serde_json::Value;

use super::model::{Dir, Exit, RoomId};

/// A room as one GMCP message described it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomInfo {
    pub id: RoomId,
    pub name: String,
    /// The key rooms are grouped by: the game's area id when it sends coordinates with
    /// one, else the area's name.
    pub area: String,
    pub area_name: String,
    pub env: String,
    /// The game's own coordinates, when it sends them: x, y, and z if it gives one.
    pub coords: Option<(i32, i32, Option<i32>)>,
    pub exits: Vec<Exit>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RoomEvent {
    Info(RoomInfo),
    /// The game refused a move ("You can't go that way"): the direction, when it said.
    WrongDir(String),
}

/// The room event in a GMCP message, if it is one Clay maps from.
pub fn parse(package: &str, json: &str) -> Option<RoomEvent> {
    let p = package.trim().to_ascii_lowercase();
    if p == "room.wrongdir" {
        let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
        let dir = match &v {
            Value::String(s) => s.clone(),
            other => get(other, "dir").and_then(as_text).unwrap_or_default(),
        };
        return Some(RoomEvent::WrongDir(dir));
    }
    if p != "room.info" {
        return None;
    }
    let v: Value = serde_json::from_str(json).ok()?;
    let id = ["num", "vnum", "id", "identifier"].iter().find_map(|k| get(&v, k).and_then(as_text)).filter(|s| !s.is_empty())?;
    let name = get(&v, "name").and_then(as_text).unwrap_or_default();
    let env = ["environment", "terrain", "env"].iter().find_map(|k| get(&v, k).and_then(as_text)).unwrap_or_default();
    let area_name = ["area", "zone"].iter().find_map(|k| get(&v, k).and_then(as_text)).unwrap_or_default();
    let (coords, area_id) = coords_of(&v);
    let area = match area_id {
        Some(a) => format!("#{a}"),
        None if !area_name.is_empty() => area_name.clone(),
        None => "?".to_string(),
    };
    let exits = exits_of(get(&v, "exits"));
    Some(RoomEvent::Info(RoomInfo { id, name: strip_codes(&name), area, area_name: strip_codes(&area_name), env, coords, exits }))
}

/// A key's value, any case.
fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let obj = v.as_object()?;
    obj.get(key).or_else(|| obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v))
}

/// A number or string as text; nothing for anything else.
fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn as_int(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n.as_i64().map(|n| n as i32).or_else(|| n.as_f64().map(|f| f.round() as i32)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A room name can carry the game's colour codes; the map shows the text.
fn strip_codes(s: &str) -> String {
    crate::util::strip_ansi_codes(s).trim().to_string()
}

/// The game's coordinates and the area id they belong to, in the forms games send:
/// IRE's `coords: "area,x,y,z"`, Aardwolf's `coord: {id, x, y, cont}`, or `x`/`y`/`z` on the
/// room itself or under `coord`/`coords`.
/// x, y and (when the game says) z.
type Coords = (i32, i32, Option<i32>);

fn coords_of(v: &Value) -> (Option<Coords>, Option<String>) {
    if let Some(Value::String(s)) = get(v, "coords") {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let nums: Vec<Option<i32>> = parts[1..].iter().map(|p| p.parse().ok()).collect();
            if let (Some(x), Some(y)) = (nums[0], nums[1]) {
                return (Some((x, y, nums.get(2).copied().flatten())), Some(parts[0].to_string()));
            }
        }
    }
    for key in ["coord", "coords"] {
        if let Some(c @ Value::Object(_)) = get(v, key) {
            let x = get(c, "x").and_then(as_int);
            let y = get(c, "y").and_then(as_int);
            let z = get(c, "z").and_then(as_int);
            let area = get(c, "id").or_else(|| get(c, "area")).and_then(as_text);
            // Aardwolf sends -1,-1 for rooms with no position (continents mark `cont`).
            if let (Some(x), Some(y)) = (x, y) {
                if x >= 0 || y >= 0 || get(c, "cont").and_then(as_int) == Some(1) {
                    return (Some((x, y, z)), area);
                }
            }
            return (None, None);
        }
    }
    if let (Some(x), Some(y)) = (get(v, "x").and_then(as_int), get(v, "y").and_then(as_int)) {
        return (Some((x, y, get(v, "z").and_then(as_int))), None);
    }
    (None, None)
}

/// The room's exits: `{dir: id}` (an id as a number, a string or an object holding one),
/// or a list of names with no targets. A target of -1, 0 or empty is not known yet.
fn exits_of(v: Option<&Value>) -> Vec<Exit> {
    let mut out = Vec::new();
    match v {
        Some(Value::Object(map)) => {
            for (k, target) in map {
                let to = match target {
                    Value::Object(_) => ["id", "num", "vnum", "to"].iter().find_map(|f| get(target, f).and_then(as_text)),
                    other => as_text(other),
                }
                .filter(|t| !t.is_empty() && t != "-1" && t != "0");
                out.push(Exit { cmd: k.trim().to_string(), dir: Dir::parse(k), to });
            }
        }
        Some(Value::Array(list)) => {
            for item in list {
                if let Some(k) = as_text(item) {
                    out.push(Exit { cmd: k.clone(), dir: Dir::parse(&k), to: None });
                }
            }
        }
        _ => {}
    }
    // The game's order isn't a map's (JSON objects have none): a fixed one, directions first.
    out.sort_by(|a, b| (a.dir.is_none(), a.dir, &a.cmd).cmp(&(b.dir.is_none(), b.dir, &b.cmd)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(package: &str, json: &str) -> RoomInfo {
        match parse(package, json) {
            Some(RoomEvent::Info(i)) => i,
            other => panic!("not a room: {other:?}"),
        }
    }

    #[test]
    fn ire_room_info() {
        let r = info(
            "Room.Info",
            r#"{"num": 12345, "name": "On a hill", "area": "Barren hills", "environment": "Hills",
                "coords": "45,5,4,3", "map": "www.achaea.com/irex/maps/clientmap.php?map=45&level=3 5 4",
                "exits": { "n": 12344, "se": 12336 }, "details": ["shop", "bank"]}"#,
        );
        assert_eq!(r.id, "12345");
        assert_eq!((r.name.as_str(), r.area.as_str(), r.area_name.as_str(), r.env.as_str()), ("On a hill", "#45", "Barren hills", "Hills"));
        assert_eq!(r.coords, Some((5, 4, Some(3))));
        assert_eq!(r.exits, vec![
            Exit { cmd: "n".into(), dir: Some(Dir::N), to: Some("12344".into()) },
            Exit { cmd: "se".into(), dir: Some(Dir::Se), to: Some("12336".into()) },
        ]);
    }

    #[test]
    fn aardwolf_room_info() {
        let r = info(
            "room.info",
            r#"{"num": 32418, "name": "@WThe Grand City of Aylor", "zone": "aylor", "terrain": "city",
                "details": "", "exits": {"n": 32419, "s": -1, "d": 35200}, "coord": {"id": 0, "x": 30, "y": 19, "cont": 0}}"#,
        );
        assert_eq!((r.id.as_str(), r.area.as_str(), r.env.as_str()), ("32418", "#0", "city"));
        assert_eq!(r.coords, Some((30, 19, None)));
        assert_eq!(r.exits.iter().map(|e| (e.cmd.as_str(), e.to.as_deref())).collect::<Vec<_>>(), [("n", Some("32419")), ("s", None), ("d", Some("35200"))]);
    }

    #[test]
    fn generic_forms_and_mixed_case() {
        let r = info("ROOM.INFO", r#"{"VNUM": "lib-3", "Name": "\u001b[1mLibrary\u001b[0m", "Area": "Town", "X": 2, "y": -1, "exits": {"North": {"id": "lib-2"}, "portal": "x9", "west": ""}}"#);
        assert_eq!((r.id.as_str(), r.name.as_str(), r.area.as_str()), ("lib-3", "Library", "Town"));
        assert_eq!(r.coords, Some((2, -1, None)));
        assert_eq!(r.exits.iter().map(|e| (e.cmd.as_str(), e.dir, e.to.as_deref())).collect::<Vec<_>>(), [("North", Some(Dir::N), Some("lib-2")), ("west", Some(Dir::W), None), ("portal", None, Some("x9"))]);
        let r = info("Room.Info", r#"{"num": 7, "exits": ["north", "climb rope"]}"#);
        assert_eq!(r.area, "?");
        assert_eq!(r.exits.iter().map(|e| (e.cmd.as_str(), e.dir)).collect::<Vec<_>>(), [("north", Some(Dir::N)), ("climb rope", None)]);
    }

    #[test]
    fn what_isnt_a_room() {
        assert_eq!(parse("Room.Info", r#"{"name": "no id"}"#), None);
        assert_eq!(parse("Char.Vitals", r#"{"hp": 1}"#), None);
        assert_eq!(parse("Room.Info", "not json"), None);
        assert_eq!(parse("Room.WrongDir", r#""ne""#), Some(RoomEvent::WrongDir("ne".into())));
    }
}
