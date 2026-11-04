//! The map's data: rooms, their exits, and where each is drawn.
//!
//! Positions are screen-like: x grows east, y grows south, z grows up. A room is placed
//! once (by the game's own coordinates when it sends them, else by the direction it was
//! entered from, `layout.rs`) and never moves on its own afterwards, so a map doesn't
//! shift under the user as it grows.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// A room's id as the game numbers it, kept as text: some games use numbers, some
/// strings, and the number 0 is a real room in a few.
pub type RoomId = String;

/// A direction a game's exits are named by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Dir {
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Nw,
    Up,
    Down,
    In,
    Out,
}

impl Dir {
    pub const ALL: [Dir; 12] = [Dir::N, Dir::Ne, Dir::E, Dir::Se, Dir::S, Dir::Sw, Dir::W, Dir::Nw, Dir::Up, Dir::Down, Dir::In, Dir::Out];

    /// "n", "north", "N", "Northeast", "u", "up"... any case; None for anything else.
    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "n" | "north" => Dir::N,
            "ne" | "northeast" | "north-east" => Dir::Ne,
            "e" | "east" => Dir::E,
            "se" | "southeast" | "south-east" => Dir::Se,
            "s" | "south" => Dir::S,
            "sw" | "southwest" | "south-west" => Dir::Sw,
            "w" | "west" => Dir::W,
            "nw" | "northwest" | "north-west" => Dir::Nw,
            "u" | "up" => Dir::Up,
            "d" | "down" => Dir::Down,
            "in" | "enter" => Dir::In,
            "out" | "leave" => Dir::Out,
            _ => return None,
        })
    }

    /// The short form a route is written in: "n", "ne", ..., "u", "d", "in", "out".
    pub fn short(self) -> &'static str {
        match self {
            Dir::N => "n",
            Dir::Ne => "ne",
            Dir::E => "e",
            Dir::Se => "se",
            Dir::S => "s",
            Dir::Sw => "sw",
            Dir::W => "w",
            Dir::Nw => "nw",
            Dir::Up => "u",
            Dir::Down => "d",
            Dir::In => "in",
            Dir::Out => "out",
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::N => Dir::S,
            Dir::Ne => Dir::Sw,
            Dir::E => Dir::W,
            Dir::Se => Dir::Nw,
            Dir::S => Dir::N,
            Dir::Sw => Dir::Ne,
            Dir::W => Dir::E,
            Dir::Nw => Dir::Se,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::In => Dir::Out,
            Dir::Out => Dir::In,
        }
    }

    /// One step's move on the grid: (east, south, up). In and Out have none.
    pub fn delta(self) -> Option<(i32, i32, i32)> {
        Some(match self {
            Dir::N => (0, -1, 0),
            Dir::Ne => (1, -1, 0),
            Dir::E => (1, 0, 0),
            Dir::Se => (1, 1, 0),
            Dir::S => (0, 1, 0),
            Dir::Sw => (-1, 1, 0),
            Dir::W => (-1, 0, 0),
            Dir::Nw => (-1, -1, 0),
            Dir::Up => (0, 0, 1),
            Dir::Down => (0, 0, -1),
            Dir::In | Dir::Out => return None,
        })
    }
}

/// One way out of a room: the command that takes it (what the game named it, what a walk
/// sends), its direction when it is one, and the room it leads to when the game said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    pub cmd: String,
    pub dir: Option<Dir>,
    /// `None`: an exit the game shows but hasn't said (or Clay hasn't seen) where to.
    pub to: Option<RoomId>,
}

/// Where a room's position came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placed {
    /// The game's own coordinates.
    Given,
    /// Beside the room it was entered from, by the direction taken.
    Inferred,
    /// Nowhere to put it next to: beside its area's rooms, apart.
    Island,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    /// The area's key (`gmcp.rs`): what groups rooms onto one map.
    pub area: String,
    /// The area's name as the game shows it.
    pub area_name: String,
    pub env: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub placed: Placed,
    /// The game's own coordinates as it sent them (x, y, z), for a Given room: kept so a
    /// change of the y axis's direction (`MapMeta::y_sign`) can place it again.
    pub given: Option<(i32, i32, i32)>,
    pub exits: Vec<Exit>,
}

impl Room {
    pub fn exit_to(&self, to: &str) -> Option<&Exit> {
        self.exits.iter().find(|e| e.to.as_deref() == Some(to))
    }

    pub fn has_dir(&self, dir: Dir) -> bool {
        self.exits.iter().any(|e| e.dir == Some(dir))
    }
}

/// What a map learned about its game, kept with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapMeta {
    /// Which way the game's y grows: 1 south (the screen's way), -1 north. Learned from
    /// moves north and south between rooms that have the game's coordinates.
    pub y_sign: i32,
    /// Moves that disagreed with `y_sign`, since the last that agreed.
    #[serde(default)]
    pub y_doubts: u32,
}

impl Default for MapMeta {
    fn default() -> Self {
        MapMeta { y_sign: 1, y_doubts: 0 }
    }
}

/// One world's map.
#[derive(Default, Debug, Clone)]
pub struct MapGraph {
    pub rooms: HashMap<RoomId, Room>,
    /// Which room is drawn in each cell, per area.
    cells: HashMap<(String, i32, i32, i32), RoomId>,
    pub meta: MapMeta,
    /// Every room may have moved (the y axis turned, `layout::vote_y`): whoever keeps a
    /// copy (the views, the store) takes them all again. Cleared by whoever acts on it.
    pub moved_all: bool,
}

impl MapGraph {
    pub fn get(&self, id: &str) -> Option<&Room> {
        self.rooms.get(id)
    }

    pub fn len(&self) -> usize {
        self.rooms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }

    /// The room drawn at a cell of an area.
    pub fn at(&self, area: &str, x: i32, y: i32, z: i32) -> Option<&RoomId> {
        self.cells.get(&(area.to_string(), x, y, z))
    }

    /// Put a room in (replacing one with its id), keeping the cell index right.
    pub fn insert(&mut self, room: Room) {
        if let Some(old) = self.rooms.get(&room.id) {
            let key = (old.area.clone(), old.x, old.y, old.z);
            if self.cells.get(&key) == Some(&old.id) {
                self.cells.remove(&key);
            }
        }
        self.cells.insert((room.area.clone(), room.x, room.y, room.z), room.id.clone());
        self.rooms.insert(room.id.clone(), room);
    }

    /// Each area's rooms' extent on one level: (min x, min y, max x, max y).
    pub fn bounds(&self, area: &str, z: i32) -> Option<(i32, i32, i32, i32)> {
        let mut it = self.rooms.values().filter(|r| r.area == area && r.z == z);
        let first = it.next()?;
        let mut b = (first.x, first.y, first.x, first.y);
        for r in it {
            b = (b.0.min(r.x), b.1.min(r.y), b.2.max(r.x), b.3.max(r.y));
        }
        Some(b)
    }

    /// An area's levels, and how many rooms each has.
    pub fn levels(&self, area: &str) -> BTreeMap<i32, usize> {
        let mut out = BTreeMap::new();
        for r in self.rooms.values().filter(|r| r.area == area) {
            *out.entry(r.z).or_insert(0) += 1;
        }
        out
    }

    /// Whether an area has any rooms yet.
    pub fn has_area(&self, area: &str) -> bool {
        self.rooms.values().any(|r| r.area == area)
    }

    /// The key of the area a user named: its key as it is, else its key or its name in
    /// any case (`/map area Barren hills` for an area the game numbers `#45`).
    pub fn area_key(&self, typed: &str) -> Option<String> {
        let typed = typed.trim();
        if self.has_area(typed) {
            return Some(typed.to_string());
        }
        self.rooms.values().find(|r| r.area.eq_ignore_ascii_case(typed) || r.area_name.eq_ignore_ascii_case(typed)).map(|r| r.area.clone())
    }
}
