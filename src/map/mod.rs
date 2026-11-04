//! The auto-mapper (docs/markdown/25-mapper.md): a map of each world built from the rooms
//! the game reports over GMCP as the user moves, kept in `~/.clay/maps.db`, drawn by every
//! interface, with routes and walks. The server owns it; clients draw what it sends.
//!
//! - `gmcp.rs` reads a room out of `Room.Info` (IRE, Aardwolf and generic shapes);
//! - `layout.rs` places it - the game's coordinates, else beside the room left;
//! - `path.rs` finds routes and rooms by name; `walk.rs` walks a route, one step per
//!   room the game reports;
//! - `wire.rs` is what clients are sent, `text.rs` the consoles' text map;
//! - `store.rs` keeps maps on disk; `app.rs` is the `App`'s side of it.

pub mod app;
pub mod gmcp;
pub mod layout;
pub mod model;
pub mod path;
pub mod store;
pub mod text;
pub mod walk;
pub mod wire;

use std::collections::HashSet;
use std::time::Instant;

use gmcp::RoomEvent;
use model::{Dir, MapGraph, RoomId};
use walk::{WalkFx, Walker};

/// One world's mapper, on the Clay that holds its connection.
#[derive(Default, Debug)]
pub struct WorldMapper {
    pub graph: MapGraph,
    /// The saved map was read in (or there was none to read).
    pub loaded: bool,
    /// The room the user is in, as the game last reported it.
    pub here: Option<RoomId>,
    pub walker: Walker,
    /// The last movement command sent, typed or walked: it explains the next room.
    pub last_sent: Option<(Dir, Instant)>,
    /// Something every map view shows changed (the user's room, the walk).
    pub dirty_state: bool,
    /// Rooms changed since the views were last sent them.
    pub dirty_rooms: HashSet<RoomId>,
    /// Rooms changed since they were last saved.
    pub unsaved: HashSet<RoomId>,
}

impl WorldMapper {
    /// A room's name and id as the user reads it: "Palatine Steps (#4134)".
    pub fn name_of(&self, id: &str) -> String {
        match self.graph.get(id) {
            Some(r) if !r.name.is_empty() => format!("{} (#{id})", r.name),
            _ => format!("#{id}"),
        }
    }

    /// A GMCP room event: the map learns the room, the user is there, and a walk takes
    /// its next step (or stops). Returns what the walk asks for.
    pub fn on_event(&mut self, ev: RoomEvent, now: Instant) -> Vec<WalkFx> {
        match ev {
            RoomEvent::WrongDir(dir) => {
                let fx = self.walker.on_wrong_dir(&dir);
                self.dirty_state |= !fx.is_empty();
                fx
            }
            RoomEvent::Info(info) => {
                let arrival = layout::Arrival { from: self.here.clone(), sent: self.last_sent };
                if layout::record(&mut self.graph, &info, &arrival, now) {
                    for id in std::iter::once(info.id.clone()).chain(arrival.from.clone()) {
                        self.dirty_rooms.insert(id.clone());
                        self.unsaved.insert(id);
                    }
                }
                // The y axis turned: every room the game placed moved.
                if std::mem::take(&mut self.graph.moved_all) {
                    let all: Vec<RoomId> = self.graph.rooms.keys().cloned().collect();
                    self.dirty_rooms.extend(all.iter().cloned());
                    self.unsaved.extend(all);
                }
                self.last_sent = None;
                self.here = Some(info.id.clone());
                self.dirty_state = true;
                let graph = &self.graph;
                let name = |id: &str| match graph.get(id) {
                    Some(r) if !r.name.is_empty() => format!("{} (#{id})", r.name),
                    _ => format!("#{id}"),
                };
                self.walker.on_room(&info.id, &name, now)
            }
        }
    }

    /// A command went to the game: a movement explains the next room reported.
    pub fn note_sent(&mut self, cmd: &str, now: Instant) {
        if let Some(dir) = Dir::parse(cmd) {
            self.last_sent = Some((dir, now));
        }
    }

    /// The route from the user's room to a room the user named; or what to tell them.
    pub fn route_to(&self, target: &str) -> Result<(RoomId, Vec<path::Step>), String> {
        if self.graph.is_empty() {
            return Err("There is no map for this world yet: Clay builds it from the rooms the game reports over GMCP (Room.Info) as you move.".to_string());
        }
        let Some(here) = self.here.as_deref() else { return Err("Clay doesn't know which room you are in yet: move, or look.".to_string()) };
        let to = match path::resolve(&self.graph, target, Some(here)) {
            path::Target::One(id) => id,
            path::Target::None => return Err(format!("No room on the map matches '{}'.", target.trim())),
            path::Target::Many(ids) => {
                let list: Vec<String> = ids.iter().map(|id| self.name_of(id)).collect();
                return Err(format!("Which room? {}. Name one, or give its #id.", list.join("; ")));
            }
        };
        match path::route(&self.graph, here, &to) {
            Some(steps) => Ok((to, steps)),
            None => Err(format!("The map knows no way from {} to {}.", self.name_of(here), self.name_of(&to))),
        }
    }
}
