//! The map as the server sends it to clients (and as the consoles draw it): rooms of one
//! area's level near a centre, and the user's place and walk.

use serde::{Deserialize, Serialize};

use super::model::{MapGraph, Room};
use super::walk::Walker;

/// Most rooms one area snapshot carries: the ones nearest its centre.
pub const SNAPSHOT_CAP: usize = 1500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapExitMsg {
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// 0: to a room on the same level of the same area; 1: to another level; 2: to another
    /// area; 3: not explored (where it leads isn't known).
    #[serde(default)]
    pub kind: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapRoomMsg {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    #[serde(default)]
    pub exits: Vec<MapExitMsg>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapStepMsg {
    pub cmd: String,
    pub to: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapWalkMsg {
    pub to: String,
    pub to_name: String,
    pub steps: Vec<MapStepMsg>,
    /// The step in flight.
    pub next: u32,
}

/// Where the user is on a world's map, and the walk going on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct MapStateMsg {
    #[serde(default)]
    pub here: Option<MapRoomMsg>,
    #[serde(default)]
    pub area: String,
    #[serde(default)]
    pub area_name: String,
    #[serde(default)]
    pub level: i32,
    #[serde(default)]
    pub rooms_total: u32,
    #[serde(default)]
    pub walk: Option<MapWalkMsg>,
    /// Why the last walk ended.
    #[serde(default)]
    pub last_stop: Option<String>,
}

/// What a client's map pane shows: a world's area and level (`None`: follow the user).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapViewReq {
    pub world_index: usize,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub level: Option<i32>,
}

/// A room found by a search.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MapRoomBrief {
    pub id: String,
    pub name: String,
    pub area_name: String,
    pub level: i32,
    /// The area's key (`MapViewReq.area`), so a client can show the room's own area and
    /// level when it is picked.
    #[serde(default)]
    pub area: String,
}

pub fn room_msg(graph: &MapGraph, room: &Room) -> MapRoomMsg {
    MapRoomMsg {
        id: room.id.clone(),
        name: room.name.clone(),
        x: room.x,
        y: room.y,
        z: room.z,
        env: room.env.clone(),
        exits: room
            .exits
            .iter()
            .map(|e| {
                let target = e.to.as_deref().and_then(|t| graph.get(t));
                let kind = match target {
                    None => 3,
                    Some(t) if t.area != room.area => 2,
                    Some(t) if t.z != room.z => 1,
                    Some(_) => 0,
                };
                MapExitMsg { cmd: e.cmd.clone(), dir: e.dir.map(|d| d.short().to_string()), to: e.to.clone(), kind }
            })
            .collect(),
    }
}

/// An area's level as sent: its rooms nearest `center` (all of them when they fit in
/// `SNAPSHOT_CAP`), and whether some were left out.
pub fn snapshot_area(graph: &MapGraph, area: &str, level: i32, center: (i32, i32)) -> (Vec<MapRoomMsg>, bool) {
    let mut rooms: Vec<&Room> = graph.rooms.values().filter(|r| r.area == area && r.z == level).collect();
    let truncated = rooms.len() > SNAPSHOT_CAP;
    rooms.sort_by_key(|r| ((r.x - center.0).abs().max((r.y - center.1).abs()), r.id.clone()));
    rooms.truncate(SNAPSHOT_CAP);
    (rooms.into_iter().map(|r| room_msg(graph, r)).collect(), truncated)
}

/// Where the user is and what the walk is doing, for a world's map.
pub fn state_msg(graph: &MapGraph, here: Option<&str>, walker: &Walker) -> MapStateMsg {
    let room = here.and_then(|h| graph.get(h));
    let name = |id: &str| graph.get(id).map(|r| r.name.clone()).unwrap_or_else(|| format!("#{id}"));
    MapStateMsg {
        here: room.map(|r| room_msg(graph, r)),
        area: room.map(|r| r.area.clone()).unwrap_or_default(),
        area_name: room.map(|r| r.area_name.clone()).unwrap_or_default(),
        level: room.map_or(0, |r| r.z),
        rooms_total: graph.len() as u32,
        walk: walker.walk.as_ref().map(|w| MapWalkMsg {
            to: w.to.clone(),
            to_name: name(&w.to),
            steps: w.steps.iter().map(|s| MapStepMsg { cmd: s.cmd.clone(), to: s.to.clone() }).collect(),
            next: w.next as u32,
        }),
        last_stop: walker.last_stop.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_the_area_level_nearest_first_and_capped() {
        let g = crate::map::path::tests_support::rome();
        let (rooms, truncated) = snapshot_area(&g, "Forum", 0, (1, 2));
        assert!(!truncated);
        assert_eq!(rooms[0].id, "F", "the centre first");
        assert_eq!(rooms.len(), 12);
        let m = rooms.iter().find(|r| r.id == "M").unwrap();
        assert_eq!(m.exits.iter().map(|e| (e.cmd.as_str(), e.kind)).collect::<Vec<_>>(), [("w", 0), ("u", 3)]);
        let json = serde_json::to_string(m).unwrap();
        let back: MapRoomMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, m);
        assert!(!json.contains("\"env\""), "empty fields stay off the wire");
    }
}
