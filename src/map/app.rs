//! The `App`'s side of the mapper: rooms from GMCP into a world's map, the map kept on
//! disk, walks stepped and timed, `/map` run, and every view told what changed.

use std::collections::HashMap;
use std::time::Instant;

use super::walk::WalkFx;
use super::wire::{self, MapViewReq};
use super::{gmcp, store::MapStore};
use crate::websocket::WsMessage;
use crate::{persistence, App, WriteCommand};

/// `/map`'s forms. The pane ones (bare, show, hide, area, level, center) are each
/// interface's own, like `/split`; the rest run on the Clay that holds the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapCmd {
    Toggle,
    Show,
    Hide,
    Area(Option<String>),
    Level(String),
    Center,
    Go(String),
    Path(String),
    Stop,
    Find(String),
    Here,
    Forget(String),
    Help,
}

impl MapCmd {
    pub fn parse(args: &str) -> MapCmd {
        let args = args.trim();
        let (verb, rest) = args.split_once(char::is_whitespace).map_or((args, ""), |(v, r)| (v, r.trim()));
        match verb.to_ascii_lowercase().as_str() {
            "" => MapCmd::Toggle,
            "show" | "on" | "open" => MapCmd::Show,
            "hide" | "off" | "close" => MapCmd::Hide,
            "area" => MapCmd::Area((!rest.is_empty()).then(|| rest.to_string())),
            "level" => MapCmd::Level(rest.to_string()),
            "center" | "centre" => MapCmd::Center,
            "go" | "walk" => MapCmd::Go(rest.to_string()),
            "path" | "route" => MapCmd::Path(rest.to_string()),
            "stop" => MapCmd::Stop,
            "find" | "search" => MapCmd::Find(rest.to_string()),
            "here" | "where" => MapCmd::Here,
            "forget" => MapCmd::Forget(rest.to_string()),
            _ => MapCmd::Help,
        }
    }

    /// A form that acts on the interface it was typed in (its map pane), not the world.
    pub fn is_pane(&self) -> bool {
        matches!(self, MapCmd::Toggle | MapCmd::Show | MapCmd::Hide | MapCmd::Area(_) | MapCmd::Level(_) | MapCmd::Center)
    }
}

/// The console's map pane (`/map`): what it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsoleMapPane {
    /// The area shown; `None` follows the user.
    pub area: Option<String>,
    /// The level shown; `None` is the user's (or the area's lowest).
    pub level: Option<i32>,
    /// A route `/map path` showed: its destination and the rooms on the way.
    pub target: Option<String>,
    pub route: Vec<String>,
}

/// The SSH console's copy of a world's map, as the server sends it (`MapState`/`MapArea`).
#[derive(Clone, Debug, Default)]
pub struct MirroredMap {
    pub state: Option<wire::MapStateMsg>,
    pub areas: HashMap<(String, i32), Vec<wire::MapRoomMsg>>,
    /// Each area's levels, as the server last listed them.
    pub levels: HashMap<String, Vec<i32>>,
    /// Each area's name by its key, as the server named them.
    pub names: HashMap<String, String>,
}

impl MirroredMap {
    /// The key of an area the user named (`/map area <name>`): the server is asked for it
    /// by that name and answers with the key, which this finds again - its key as it is,
    /// else a key or name in any case. Unknown: the name itself.
    pub fn area_key(&self, typed: &str) -> String {
        let typed = typed.trim();
        if self.levels.contains_key(typed) || self.areas.keys().any(|(a, _)| a == typed) || self.state.as_ref().is_some_and(|s| s.area == typed) {
            return typed.to_string();
        }
        let named = |key: &str, name: &str| key.eq_ignore_ascii_case(typed) || name.eq_ignore_ascii_case(typed);
        if let Some(s) = self.state.as_ref().filter(|s| named(&s.area, &s.area_name)) {
            return s.area.clone();
        }
        self.names.iter().find(|(k, n)| named(k, n)).map_or_else(|| typed.to_string(), |(k, _)| k.clone())
    }

    /// The server's name for an area (a whole area's `MapArea` carries it).
    pub fn note_area_name(&mut self, area: &str, name: &str) {
        if !name.is_empty() {
            self.names.insert(area.to_string(), name.to_string());
        }
    }

    /// An area's level from the server: all of it (`replace`), or the rooms that changed.
    pub fn apply_area(&mut self, area: String, level: i32, replace: bool, rooms: Vec<wire::MapRoomMsg>) {
        let list = self.areas.entry((area, level)).or_default();
        if replace {
            *list = rooms;
            return;
        }
        for room in rooms {
            match list.iter_mut().find(|r| r.id == room.id) {
                Some(old) => *old = room,
                None => list.push(room),
            }
        }
    }
}

pub const MAP_HELP: &str = "/map: show or hide the map\n\
/map go <room or #id>: walk there, one step per room the game reports\n\
/map path <room or #id>: show the route, without walking it\n\
/map stop: stop walking (Esc does too)\n\
/map find <text>: rooms whose name has it\n\
/map here: the room you are in\n\
/map area [name] | level <n|up|down> | center: what the map pane shows\n\
/map forget <world>: delete that world's map";

impl App {
    /// The maps file, opened the first time a map is needed (a user who never meets a
    /// game with room data gets none).
    pub(crate) fn map_store(&mut self) -> Option<&MapStore> {
        if self.maps.is_none() && !self.maps_tried {
            self.maps_tried = true;
            self.maps = MapStore::open(crate::clay_config_path("maps.db"));
            // Deleted worlds' maps go after a while (store.rs `ORPHAN_KEEP_MS`).
            let live: Vec<String> = self.worlds.iter().map(|w| w.world_id.clone()).filter(|id| !id.is_empty()).collect();
            if let Some(store) = &self.maps {
                store.purge_orphans(live);
            }
        }
        self.maps.as_ref()
    }

    /// A world's saved map, read in the first time it is needed. The world gets its
    /// lasting id here if it had none (the map is kept under it), saved at once.
    fn map_ensure_loaded(&mut self, world_idx: usize) {
        if self.worlds.get(world_idx).is_none_or(|w| w.map.loaded) {
            return;
        }
        self.worlds[world_idx].map.loaded = true;
        let minted = self.worlds[world_idx].world_id.is_empty();
        let uuid = self.worlds[world_idx].ensure_world_id().to_string();
        if minted {
            let _ = persistence::save_settings(self);
        }
        let Some((rooms, meta)) = self.map_store().and_then(|s| s.load(&uuid)) else { return };
        let map = &mut self.worlds[world_idx].map;
        map.graph.meta = meta;
        for room in rooms {
            map.graph.insert(room);
        }
        map.dirty_state = true;
    }

    /// A connection that agreed GMCP without asking for room reports (made before `Room 1`
    /// joined the default packages, or kept across the reload that brought it) is asked
    /// now, once - when the user first wants its map. Only when the world's GMCP setting
    /// lists `Room`.
    pub(crate) fn map_ask_for_rooms(&mut self, world_idx: usize) {
        let Some(w) = self.worlds.get_mut(world_idx) else { return };
        let wants = w.settings.gmcp_packages.split(',').any(|p| crate::stats::package_has_prefix(p.trim(), "room"));
        let has = w.protocol.gmcp_supported_packages.iter().any(|p| crate::stats::package_has_prefix(p, "room"));
        if !w.protocol.gmcp_enabled || !wants || has {
            return;
        }
        let Some(tx) = w.command_tx.as_ref() else { return };
        let add = crate::telnet::build_gmcp_message("Core.Supports.Add", r#"["Room 1"]"#);
        if tx.try_send(WriteCommand::Raw(add)).is_ok() {
            w.protocol.gmcp_supported_packages.push("Room 1".to_string());
        }
    }

    /// GMCP from a world (`handle_telnet_event`): a room report for its map. Recorded
    /// whatever F9 says - F9 is the game's media, the map is Clay's.
    pub(crate) fn map_on_gmcp(&mut self, world_idx: usize, package: &str, json: &str) {
        let Some(ev) = gmcp::parse(package, json) else { return };
        if world_idx >= self.worlds.len() {
            return;
        }
        self.map_ensure_loaded(world_idx);
        let fx = self.worlds[world_idx].map.on_event(ev, Instant::now());
        self.map_apply_fx(world_idx, fx);
    }

    /// What a walk asked for: a step sent (a failed send ends the walk), a line said.
    fn map_apply_fx(&mut self, world_idx: usize, fx: Vec<WalkFx>) {
        for f in fx {
            match f {
                WalkFx::Send(cmd) => {
                    if !self.send_walk_step(world_idx, &cmd) {
                        let more = self.worlds[world_idx].map.walker.stop("Walk stopped: the step couldn't be sent.");
                        self.worlds[world_idx].map.dirty_state = true;
                        self.map_apply_fx(world_idx, more);
                    }
                }
                WalkFx::Say(text) => {
                    self.worlds[world_idx].map.dirty_state = true;
                    let headless = !self.console_tui;
                    self.emit_client_text(world_idx, &text, headless);
                }
            }
        }
    }

    /// One step of a walk to the game. Unlike `send_to_world`, whose answer is true even
    /// when the queue refused the line, this says whether it really went.
    fn send_walk_step(&mut self, world_idx: usize, cmd: &str) -> bool {
        let Some(tx) = self.worlds.get(world_idx).and_then(|w| w.command_tx.clone()) else { return false };
        if tx.try_send(WriteCommand::Text(cmd.to_string())).is_err() {
            return false;
        }
        self.tf_echo_line(world_idx, "secho", "sprefix", cmd);
        self.capture_sent_line(world_idx, cmd);
        self.worlds[world_idx].last_send_time = Some(Instant::now());
        true
    }

    /// When the first walk step still waiting runs out of time.
    pub(crate) fn next_walk_deadline(&self) -> Option<Instant> {
        self.worlds.iter().filter_map(|w| w.map.walker.deadline()).min()
    }

    /// Walks whose step waited too long end, saying so.
    pub(crate) fn map_expire_walks(&mut self) {
        let now = Instant::now();
        for i in 0..self.worlds.len() {
            let fx = self.worlds[i].map.walker.tick(now);
            if !fx.is_empty() {
                self.map_apply_fx(i, fx);
            }
        }
    }

    /// A world's connection ended: its walk ends with it.
    pub(crate) fn map_world_disconnected(&mut self, world_idx: usize) {
        let Some(w) = self.worlds.get_mut(world_idx) else { return };
        let fx = w.map.walker.stop("Walk stopped: disconnected.");
        if !fx.is_empty() {
            self.map_apply_fx(world_idx, fx);
        }
    }

    /// Start walking a world to the room the user named; `Err` is what to tell them.
    pub(crate) fn map_walk(&mut self, world_idx: usize, target: &str) -> Result<(), String> {
        let Some(world) = self.worlds.get(world_idx) else { return Err("No such world.".into()) };
        if !world.connected || world.command_tx.is_none() {
            return Err(format!("{} isn't connected.", world.name));
        }
        self.map_ensure_loaded(world_idx);
        let (to, steps) = self.worlds[world_idx].map.route_to(target)?;
        let here = self.worlds[world_idx].map.here.clone().unwrap_or_default();
        let map = &mut self.worlds[world_idx].map;
        let graph = &map.graph;
        let name = |id: &str| match graph.get(id) {
            Some(r) if !r.name.is_empty() => format!("{} (#{id})", r.name),
            _ => format!("#{id}"),
        };
        let fx = map.walker.start(&here, to, steps, &name, Instant::now());
        map.dirty_state = true;
        self.map_apply_fx(world_idx, fx);
        Ok(())
    }

    /// Stop a world's walk (Esc, Stop, `/map stop`). False when there was none.
    pub(crate) fn map_stop(&mut self, world_idx: usize) -> bool {
        let Some(w) = self.worlds.get_mut(world_idx) else { return false };
        let fx = w.map.walker.stop("Walk stopped.");
        let stopped = !fx.is_empty();
        self.map_apply_fx(world_idx, fx);
        stopped
    }

    /// `/map`'s world forms, for the world shown where it was typed: the lines to tell
    /// the user. The pane forms belong to the interface; asked here they get its help.
    pub(crate) fn map_command(&mut self, world_idx: usize, cmd: &MapCmd) -> Vec<String> {
        if world_idx >= self.worlds.len() {
            return vec!["No world to map.".to_string()];
        }
        self.map_ensure_loaded(world_idx);
        self.map_ask_for_rooms(world_idx);
        match cmd {
            MapCmd::Go(target) if target.trim().is_empty() => vec!["Usage: /map go <room name or #id>".to_string()],
            MapCmd::Go(target) => match self.map_walk(world_idx, target) {
                Ok(()) => Vec::new(),
                Err(e) => vec![e],
            },
            MapCmd::Path(target) if target.trim().is_empty() => vec!["Usage: /map path <room name or #id>".to_string()],
            MapCmd::Path(target) => {
                let map = &self.worlds[world_idx].map;
                match map.route_to(target) {
                    Ok((to, steps)) if steps.is_empty() => vec![format!("You are already in {}.", map.name_of(&to))],
                    Ok((to, steps)) => {
                        let route: Vec<&str> = steps.iter().map(|s| s.cmd.as_str()).collect();
                        vec![format!("Route to {}: {} ({} steps). /map go walks it.", map.name_of(&to), route.join(", "), steps.len())]
                    }
                    Err(e) => vec![e],
                }
            }
            MapCmd::Stop => {
                if self.map_stop(world_idx) {
                    Vec::new()
                } else {
                    vec!["No walk to stop.".to_string()]
                }
            }
            MapCmd::Find(text) if text.trim().is_empty() => vec!["Usage: /map find <text>".to_string()],
            MapCmd::Find(text) => {
                let map = &self.worlds[world_idx].map;
                let found = super::path::search(&map.graph, text, map.here.as_deref(), 20);
                if found.is_empty() {
                    vec![format!("No room on the map matches '{}'.", text.trim())]
                } else {
                    std::iter::once(format!("Rooms matching '{}':", text.trim()))
                        .chain(found.iter().map(|id| {
                            let r = map.graph.get(id);
                            format!("  {} - {}", map.name_of(id), r.map(|r| r.area_name.as_str()).unwrap_or(""))
                        }))
                        .collect()
                }
            }
            MapCmd::Here => {
                let map = &self.worlds[world_idx].map;
                match map.here.as_deref().and_then(|h| map.graph.get(h)) {
                    Some(r) => {
                        let exits: Vec<&str> = r.exits.iter().map(|e| e.cmd.as_str()).collect();
                        vec![format!("{} - {}, level {}. Exits: {}. {} rooms mapped.", map.name_of(&r.id), r.area_name, r.z, if exits.is_empty() { "none".to_string() } else { exits.join(", ") }, map.graph.len())]
                    }
                    None if map.graph.is_empty() => vec!["There is no map for this world yet: Clay builds it from the rooms the game reports over GMCP (Room.Info) as you move.".to_string()],
                    None => vec![format!("Clay doesn't know which room you are in yet. {} rooms mapped.", map.graph.len())],
                }
            }
            MapCmd::Forget(name) => {
                let Some(i) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name.trim())) else {
                    return vec!["Usage: /map forget <world name> - the whole map of that world is deleted.".to_string()];
                };
                let uuid = self.worlds[i].world_id.clone();
                let name = self.worlds[i].name.clone();
                let fx = self.worlds[i].map.walker.stop("Walk stopped: the map was deleted.");
                self.map_apply_fx(i, fx);
                let map = &mut self.worlds[i].map;
                *map = super::WorldMapper { loaded: true, dirty_state: true, ..Default::default() };
                if !uuid.is_empty() {
                    if let Some(store) = self.map_store() {
                        store.forget(&uuid);
                    }
                }
                vec![format!("The map of {name} is deleted.")]
            }
            MapCmd::Help => MAP_HELP.lines().map(str::to_string).collect(),
            MapCmd::Toggle | MapCmd::Show | MapCmd::Hide | MapCmd::Area(_) | MapCmd::Level(_) | MapCmd::Center => {
                vec!["The map pane belongs to the interface you typed this in, and this one has none.".to_string()]
            }
        }
    }

    /// Changed rooms saved; every map view told what changed. True when the console's own
    /// map pane must be drawn again.
    pub(crate) fn flush_map(&mut self) -> bool {
        let mut console = false;
        for i in 0..self.worlds.len() {
            let map = &mut self.worlds[i].map;
            if !map.dirty_state && map.dirty_rooms.is_empty() && map.unsaved.is_empty() {
                continue;
            }
            if !map.unsaved.is_empty() {
                let rooms: Vec<_> = map.unsaved.drain().filter_map(|id| map.graph.get(&id).cloned()).collect();
                let meta = map.graph.meta.clone();
                let uuid = self.worlds[i].world_id.clone();
                let name = self.worlds[i].name.clone();
                if !uuid.is_empty() {
                    if let Some(store) = self.map_store() {
                        store.upsert(&uuid, &name, rooms, meta);
                    }
                }
            }
            console |= self.map_pane_world() == Some(i);
            self.map_send_views(i);
            let map = &mut self.worlds[i].map;
            map.dirty_state = false;
            map.dirty_rooms.clear();
        }
        console
    }

    /// The world the console's map pane shows: the console's own (the focused pane's).
    pub(crate) fn map_pane_world(&self) -> Option<usize> {
        self.map_pane.as_ref().map(|_| self.current_world_index)
    }

    /// The area, level and centre a client's view of a world shows: what it asked for,
    /// else where the user is. `None` when it follows the user and the user's room isn't
    /// known yet.
    fn map_view_target(&self, view: &MapViewReq) -> Option<(String, i32, (i32, i32))> {
        let map = &self.worlds.get(view.world_index)?.map;
        let here = map.here.as_deref().and_then(|h| map.graph.get(h));
        // By key, or by the name a user typed (`/map area <name>` in a client).
        let area = match &view.area {
            Some(a) => map.graph.area_key(a).unwrap_or_else(|| a.clone()),
            None => here?.area.clone(),
        };
        let level = view.level.or_else(|| here.filter(|h| h.area == area).map(|h| h.z)).or_else(|| map.graph.levels(&area).keys().next().copied()).unwrap_or(0);
        let center = match here.filter(|h| h.area == area && h.z == level) {
            Some(h) => (h.x, h.y),
            None => map.graph.bounds(&area, level).map_or((0, 0), |(x0, y0, x1, y1)| ((x0 + x1) / 2, (y0 + y1) / 2)),
        };
        Some((area, level, center))
    }

    /// An area's level, whole, to one client.
    fn map_send_area(&mut self, client_id: u64, world_idx: usize, area: &str, level: i32, center: (i32, i32)) {
        let map = &self.worlds[world_idx].map;
        let (rooms, truncated) = wire::snapshot_area(&map.graph, area, level, center);
        let area_name = map.graph.rooms.values().find(|r| r.area == area).map(|r| r.area_name.clone()).unwrap_or_default();
        let levels = map.graph.levels(area).keys().copied().collect();
        self.ws_send_to_client(client_id, WsMessage::MapArea { world_index: world_idx, area: area.to_string(), area_name, level, levels, replace: true, truncated, rooms });
        self.ws_client_map_sent.insert((client_id, world_idx, area.to_string(), level));
    }

    /// A world's map changed: to each client whose pane shows it, the user's place and walk
    /// (when they changed), and the rooms that changed in the area it shows - or the whole
    /// area, when the view moved to one it hasn't been sent.
    fn map_send_views(&mut self, world_idx: usize) {
        let watchers: Vec<(u64, MapViewReq)> = self
            .ws_client_map_views
            .iter()
            .flat_map(|(c, views)| views.iter().filter(|v| v.world_index == world_idx).map(|v| (*c, v.clone())))
            .collect();
        if watchers.is_empty() {
            return;
        }
        let map = &self.worlds[world_idx].map;
        let state = map.dirty_state.then(|| wire::state_msg(&map.graph, map.here.as_deref(), &map.walker));
        let dirty: Vec<String> = map.dirty_rooms.iter().cloned().collect();
        let mut told_state = std::collections::HashSet::new();
        for (client, view) in watchers {
            if let Some(state) = &state {
                if told_state.insert(client) {
                    self.ws_send_to_client(client, WsMessage::MapState { world_index: world_idx, state: Some(state.clone()) });
                }
            }
            let Some((area, level, center)) = self.map_view_target(&view) else { continue };
            if !self.ws_client_map_sent.contains(&(client, world_idx, area.clone(), level)) {
                self.map_send_area(client, world_idx, &area, level, center);
                continue;
            }
            let map = &self.worlds[world_idx].map;
            let rooms: Vec<_> = dirty.iter().filter_map(|id| map.graph.get(id)).filter(|r| r.area == area && r.z == level).map(|r| wire::room_msg(&map.graph, r)).collect();
            if !rooms.is_empty() {
                self.ws_send_to_client(client, WsMessage::MapArea { world_index: world_idx, area, area_name: String::new(), level, levels: Vec::new(), replace: false, truncated: false, rooms });
            }
        }
    }

    /// `MapWatch`: what a client's map panes show now. Each view is sent at once - the
    /// user's place and the whole area - and from then on what changes.
    pub(crate) fn map_watch(&mut self, client_id: u64, views: Vec<MapViewReq>) {
        self.ws_client_maps.insert(client_id);
        self.ws_client_map_sent.retain(|(c, ..)| *c != client_id);
        for view in &views {
            if view.world_index < self.worlds.len() {
                self.map_ensure_loaded(view.world_index);
                self.map_ask_for_rooms(view.world_index);
            }
        }
        self.ws_client_map_views.insert(client_id, views.clone());
        for view in views {
            let Some(w) = self.worlds.get(view.world_index) else { continue };
            let state = wire::state_msg(&w.map.graph, w.map.here.as_deref(), &w.map.walker);
            self.ws_send_to_client(client_id, WsMessage::MapState { world_index: view.world_index, state: Some(state) });
            if let Some((area, level, center)) = self.map_view_target(&view) {
                self.map_send_area(client_id, view.world_index, &area, level, center);
            }
        }
    }

    /// A client gone: nothing of its map views is kept.
    pub(crate) fn map_forget_client(&mut self, client_id: u64) {
        self.ws_client_maps.remove(&client_id);
        self.ws_client_map_views.remove(&client_id);
        self.ws_client_map_sent.retain(|(c, ..)| *c != client_id);
    }

    /// `MapRoute`: a room picked on a client's map, or typed in its search - walk there, or
    /// just answer the route.
    pub(crate) fn map_route_request(&mut self, client_id: u64, world_idx: usize, request_id: u64, to: &str, walk: bool) {
        if world_idx >= self.worlds.len() {
            return;
        }
        self.map_ensure_loaded(world_idx);
        let reply = |error: Option<String>, to: String, room, route| WsMessage::MapRouteResult { world_index: world_idx, request_id, to, room, route, error };
        if walk {
            if let Err(e) = self.map_walk(world_idx, to) {
                self.ws_tell(client_id, world_idx, &e);
                self.ws_send_to_client(client_id, reply(Some(e), String::new(), None, Vec::new()));
            }
            return;
        }
        let map = &self.worlds[world_idx].map;
        let msg = match map.route_to(to) {
            Ok((id, steps)) => {
                let room = map.graph.get(&id).map(|r| wire::room_msg(&map.graph, r));
                let route = steps.into_iter().map(|s| wire::MapStepMsg { cmd: s.cmd, to: s.to }).collect();
                reply(None, id, room, route)
            }
            Err(e) => {
                // Not reachable, but on the map: the room itself, so the card can show it.
                let id = match super::path::resolve(&map.graph, to, map.here.as_deref()) {
                    super::path::Target::One(id) => id,
                    _ => String::new(),
                };
                let room = map.graph.get(&id).map(|r| wire::room_msg(&map.graph, r));
                reply(Some(e), id, room, Vec::new())
            }
        };
        self.ws_send_to_client(client_id, msg);
    }

    /// `MapSearch`: rooms whose name matches, best first.
    pub(crate) fn map_search(&mut self, client_id: u64, world_idx: usize, request_id: u64, query: &str) {
        if world_idx >= self.worlds.len() {
            return;
        }
        self.map_ensure_loaded(world_idx);
        let map = &self.worlds[world_idx].map;
        let rooms = super::path::search(&map.graph, query, map.here.as_deref(), 20)
            .into_iter()
            .filter_map(|id| map.graph.get(&id))
            .map(|r| wire::MapRoomBrief { id: r.id.clone(), name: r.name.clone(), area_name: r.area_name.clone(), level: r.z, area: r.area.clone() })
            .collect();
        self.ws_send_to_client(client_id, WsMessage::MapSearchResult { world_index: world_idx, request_id, rooms });
    }

    /// `/map` typed in a client (either server handler): the pane forms go back to the
    /// client, which has the pane (or it is told it has none); the rest run here and the
    /// client is told.
    pub(crate) fn ws_map_command(&mut self, client_id: u64, world_index: usize, args: &str, command: &str) {
        let cmd = MapCmd::parse(args);
        if cmd.is_pane() {
            if self.ws_client_maps.contains(&client_id) {
                self.ws_send_to_client(client_id, crate::websocket::WsMessage::ExecuteLocalCommand { command: command.trim().to_string() });
            } else {
                self.ws_tell(client_id, world_index, "This interface has no map pane: update it (/update). /map go, path, find and here work everywhere.");
            }
            return;
        }
        for line in self.map_command(world_index, &cmd) {
            self.ws_tell(client_id, world_index, &line);
        }
    }

    /// `/map` typed in this console: its pane, or the world it shows.
    pub(crate) fn console_map_command(&mut self, args: &str) {
        let cmd = MapCmd::parse(args);
        // `/map path` with the pane open shows the route on it too.
        if let (MapCmd::Path(target), Some(_)) = (&cmd, &self.map_pane) {
            if let Ok((to, steps)) = self.worlds.get(self.current_world_index).map_or(Err(String::new()), |w| w.map.route_to(target)) {
                if let Some(pane) = self.map_pane.as_mut() {
                    pane.target = Some(to);
                    pane.route = steps.iter().map(|s| s.to.clone()).collect();
                }
            }
        }
        let lines = if cmd.is_pane() { self.console_map_pane(&cmd) } else { self.map_command(self.current_world_index, &cmd) };
        for line in lines {
            self.add_output(&line);
        }
    }

    /// The console's map pane: open, close, or what it shows. Quiet when it works (the pane
    /// shows it); only a problem is said.
    pub(crate) fn console_map_pane(&mut self, cmd: &MapCmd) -> Vec<String> {
        let was_open = self.map_pane.is_some();
        match cmd {
            MapCmd::Toggle if was_open => self.map_pane = None,
            MapCmd::Hide => self.map_pane = None,
            MapCmd::Toggle | MapCmd::Show => {
                self.map_pane.get_or_insert_with(Default::default);
            }
            MapCmd::Center => self.map_pane = Some(ConsoleMapPane::default()),
            MapCmd::Area(name) => {
                let area = match name {
                    None => None,
                    Some(name) => match self.console_area_named(name) {
                        Some(a) => Some(a),
                        None => return vec![format!("No area on the map is called '{name}'.")],
                    },
                };
                let pane = self.map_pane.get_or_insert_with(Default::default);
                pane.area = area;
                pane.level = None;
            }
            MapCmd::Level(how) => {
                let shown = self.console_map_shown().map_or(0, |(_, level)| level);
                let level = match how.trim().to_ascii_lowercase().as_str() {
                    "up" | "u" => shown + 1,
                    "down" | "d" => shown - 1,
                    n => match n.parse::<i32>() {
                        Ok(n) => n,
                        Err(_) => return vec!["Usage: /map level <n|up|down>".to_string()],
                    },
                };
                self.map_pane.get_or_insert_with(Default::default).level = Some(level);
            }
            _ => return Vec::new(),
        }
        if self.map_pane.is_some() && self.is_master {
            let w = self.current_world_index;
            self.map_ensure_loaded(w);
            self.map_ask_for_rooms(w);
        }
        self.needs_output_redraw = true;
        Vec::new()
    }

    /// An area's key from its name (or key) as the user typed it.
    fn console_area_named(&self, name: &str) -> Option<String> {
        let w = self.worlds.get(self.current_world_index)?;
        let name = name.trim();
        if self.is_master {
            let mut areas: Vec<(&String, &String)> = w.map.graph.rooms.values().map(|r| (&r.area, &r.area_name)).collect();
            areas.sort();
            areas.dedup();
            return areas.iter().find(|(key, n)| key.eq_ignore_ascii_case(name) || n.eq_ignore_ascii_case(name)).map(|(k, _)| k.to_string());
        }
        // Attached: the server knows every area; it is asked by this name and its answer
        // found again by `MirroredMap::area_key`.
        Some(w.mirrored_map.area_key(name))
    }

    /// Esc in the console: a walk in the world shown stops (the key goes on as ever).
    pub(crate) fn console_esc_stops_walk(&mut self) {
        let w = self.current_world_index;
        if self.is_master {
            if self.worlds.get(w).is_some_and(|w| w.map.walker.walking()) {
                self.map_stop(w);
            }
        } else if self.worlds.get(w).and_then(|w| w.mirrored_map.state.as_ref()).is_some_and(|s| s.walk.is_some()) {
            self.map_outbox.push(WsMessage::MapStop { world_index: w });
        }
    }

    /// Rows the map pane takes at the top of an output area of `total` rows, its bar
    /// included: two fifths of it, 5 to 25 map rows, never leaving the output under 4. A
    /// closed pane, or a terminal too short for one, takes none. Depends on nothing but
    /// these (never the map's contents), so a room arriving never resizes the output.
    pub(crate) fn console_map_block(&self, total: u16) -> u16 {
        if self.map_pane.is_none() {
            return 0;
        }
        let rows = (total * 2 / 5).clamp(5, 25).min(total.saturating_sub(5));
        if rows < 3 { 0 } else { rows + 1 }
    }

    /// The area and level the console's map pane shows.
    fn console_map_shown(&self) -> Option<(String, i32)> {
        let pane = self.map_pane.as_ref()?;
        let w = self.worlds.get(self.current_world_index)?;
        let (here_area, here_level) = if self.is_master {
            let here = w.map.here.as_deref().and_then(|h| w.map.graph.get(h));
            (here.map(|h| h.area.clone()), here.map(|h| h.z))
        } else {
            let s = w.mirrored_map.state.as_ref();
            (s.filter(|s| s.here.is_some()).map(|s| s.area.clone()), s.and_then(|s| s.here.as_ref().map(|h| h.z)))
        };
        let typed = if self.is_master { pane.area.clone() } else { pane.area.as_deref().map(|a| w.mirrored_map.area_key(a)) };
        let area = typed.or(here_area.clone())?;
        let level = pane.level.or_else(|| here_level.filter(|_| Some(&area) == here_area.as_ref())).unwrap_or(0);
        Some((area, level))
    }

    /// The console's map pane as text: `rows` lines `cols` wide, and its bar.
    pub(crate) fn console_map_lines(&self, cols: usize, rows: usize) -> (Vec<String>, String) {
        let Some(w) = self.worlds.get(self.current_world_index) else { return (Vec::new(), String::new()) };
        let empty = format!("No map for {} yet: Clay builds it from the rooms the game reports over GMCP as you move.", w.name);
        let shown = self.console_map_shown();
        let (rooms, state) = match (&shown, self.is_master) {
            (Some((area, level)), true) => {
                let center = w.map.here.as_deref().and_then(|h| w.map.graph.get(h)).filter(|h| &h.area == area && h.z == *level).map(|h| (h.x, h.y));
                let center = center.or_else(|| w.map.graph.bounds(area, *level).map(|(x0, y0, x1, y1)| ((x0 + x1) / 2, (y0 + y1) / 2))).unwrap_or((0, 0));
                (wire::snapshot_area(&w.map.graph, area, *level, center).0, Some(wire::state_msg(&w.map.graph, w.map.here.as_deref(), &w.map.walker)))
            }
            (Some((area, level)), false) => (w.mirrored_map.areas.get(&(area.clone(), *level)).cloned().unwrap_or_default(), w.mirrored_map.state.clone()),
            (None, true) => (Vec::new(), Some(wire::state_msg(&w.map.graph, w.map.here.as_deref(), &w.map.walker))),
            (None, false) => (Vec::new(), w.mirrored_map.state.clone()),
        };
        let here = state.as_ref().and_then(|s| s.here.as_ref()).map(|h| h.id.clone());
        let pane = self.map_pane.clone().unwrap_or_default();
        // The walk going on, else the route `/map path` showed.
        let (target, route) = match state.as_ref().and_then(|s| s.walk.as_ref()) {
            Some(walk) => (Some(walk.to.clone()), walk.steps.iter().skip(walk.next as usize).map(|s| s.to.clone()).collect()),
            None => (pane.target.clone(), pane.route.clone()),
        };
        let center = here.as_deref().and_then(|h| rooms.iter().find(|r| r.id == h)).map(|r| (r.x, r.y)).unwrap_or_else(|| {
            let xs = rooms.iter().map(|r| r.x);
            let ys = rooms.iter().map(|r| r.y);
            match (xs.clone().min(), xs.max(), ys.clone().min(), ys.max()) {
                (Some(x0), Some(x1), Some(y0), Some(y1)) => ((x0 + x1) / 2, (y0 + y1) / 2),
                _ => (0, 0),
            }
        });
        let view = super::text::View { rooms: &rooms, here: here.as_deref(), target: target.as_deref(), route: &route, center };
        let lines = super::text::render(&view, cols, rows, &empty);
        let mut bar = super::text::bar_text(&w.name, state.as_ref());
        if let Some((area, level)) = shown.filter(|_| pane.area.is_some() || pane.level.is_some()) {
            let name = if self.is_master {
                w.map.graph.rooms.values().find(|r| r.area == area).map(|r| r.area_name.clone()).filter(|n| !n.is_empty())
            } else {
                w.mirrored_map.names.get(&area).cloned()
            };
            bar.push_str(&format!(" · showing {} level {level} (/map center follows you)", name.unwrap_or(area)));
        }
        (lines, bar)
    }

    /// The SSH console's map views as the server should know them, when they changed
    /// since last said: `MapWatch` with the pane's view (an empty list: no pane open).
    pub(crate) fn remote_map_watch_due(&mut self) -> Option<WsMessage> {
        if !self.server_map {
            return None;
        }
        let views: Vec<MapViewReq> = self.map_pane.as_ref().map(|p| MapViewReq { world_index: self.current_world_index, area: p.area.clone(), level: p.level }).into_iter().collect();
        if self.map_watch_sent.as_ref() == Some(&views) {
            return None;
        }
        self.map_watch_sent = Some(views.clone());
        Some(WsMessage::MapWatch { views })
    }

    /// Everything saved, before a reload or quit.
    pub(crate) fn flush_map_store(&mut self) {
        self.flush_map();
        if let Some(store) = &self.maps {
            store.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telnet::TelnetEvent;
    use crate::World;

    fn room(app: &mut App, json: &str) {
        app.handle_telnet_event(0, &TelnetEvent::GmcpMessage("Room.Info".to_string(), json.to_string()));
    }

    fn sent(rx: &mut tokio::sync::mpsc::Receiver<WriteCommand>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(WriteCommand::Text(t)) = rx.try_recv() {
            out.push(t);
        }
        out
    }

    fn said(app: &App) -> Vec<String> {
        app.worlds[0].output_lines.iter().map(|l| l.text.clone()).collect()
    }

    /// The map builds from the rooms the game reports as the user moves; `/map go` walks
    /// back one step per room reported; a room off the route stops the walk; the map is
    /// saved and read back by the next Clay.
    #[test]
    fn rooms_from_gmcp_a_walk_step_by_step_and_the_map_kept() {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("Rome");
        w.connected = true;
        app.worlds.push(w);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WriteCommand>(16);
        app.worlds[0].command_tx = Some(tx);

        room(&mut app, r#"{"num":"F","name":"The Forum","area":"Forum","exits":{"e":"G","s":"J"}}"#);
        app.send_to_world(0, "e".into());
        room(&mut app, r#"{"num":"G","name":"Via Sacra","area":"Forum","exits":{"w":"F","e":"H"}}"#);
        app.send_to_world(0, "e".into());
        room(&mut app, r#"{"num":"H","name":"Via Sacra, East","area":"Forum","exits":{"w":"G","s":"K"}}"#);
        app.send_to_world(0, "s".into());
        room(&mut app, r#"{"num":"K","name":"Temple of Vesta","area":"Forum","exits":{"n":"H"}}"#);
        sent(&mut rx);
        let g = &app.worlds[0].map.graph;
        assert_eq!(g.len(), 4);
        assert_eq!(["F", "G", "H", "K"].map(|id| (g.get(id).unwrap().x, g.get(id).unwrap().y)), [(0, 0), (1, 0), (2, 0), (2, 1)]);

        assert!(app.map_command(0, &MapCmd::Go("the forum".into())).is_empty());
        assert_eq!(sent(&mut rx), ["n"], "one step, then wait");
        assert!(said(&app).iter().any(|l| l.starts_with("Walking to The Forum (#F): n, w, w (3 steps).")));
        assert!(app.next_walk_deadline().is_some());
        room(&mut app, r#"{"num":"H","name":"Via Sacra, East","area":"Forum","exits":{"w":"G","s":"K"}}"#);
        assert_eq!(sent(&mut rx), ["w"]);
        room(&mut app, r#"{"num":"G","name":"Via Sacra","area":"Forum","exits":{"w":"F","e":"H"}}"#);
        assert_eq!(sent(&mut rx), ["w"]);
        room(&mut app, r#"{"num":"F","name":"The Forum","area":"Forum","exits":{"e":"G","s":"J"}}"#);
        assert_eq!(sent(&mut rx), Vec::<String>::new());
        assert!(said(&app).iter().any(|l| l == "Arrived at The Forum (#F)."));
        assert!(app.next_walk_deadline().is_none());

        // Off the route: the walk stops and says where the user is.
        app.map_command(0, &MapCmd::Go("#K".into()));
        assert_eq!(sent(&mut rx), ["e"]);
        room(&mut app, r#"{"num":"X","name":"A Dark Alley","area":"Forum","exits":{}}"#);
        assert!(said(&app).iter().any(|l| l == "Walk stopped: expected Via Sacra (#G) after 'e'; you are in A Dark Alley (#X)."));
        assert_eq!(app.map_command(0, &MapCmd::Stop), ["No walk to stop."]);
        assert!(app.map_command(0, &MapCmd::Path("vesta".into()))[0].starts_with("The map knows no way"), "X has no exits");

        // Kept: another Clay reads the map back by the world's id.
        app.flush_map_store();
        let uuid = app.worlds[0].world_id.clone();
        assert!(!uuid.is_empty(), "the world got its lasting id");
        let mut next = App::new();
        next.worlds.clear();
        let mut w = World::new("Rome renamed");
        w.world_id = uuid;
        next.worlds.push(w);
        next.map_ensure_loaded(0);
        assert_eq!(next.worlds[0].map.graph.len(), 5);
        assert_eq!(next.worlds[0].map.graph.get("K").map(|r| (r.x, r.y)), Some((2, 1)));
    }

    /// The console's map pane: `/map` opens it at the top of the output area, the panes
    /// below move down by its rows, and it draws you, the rooms and its bar; `/map path`
    /// shows the route on it; `/map` again closes it and the output takes its rows back.
    #[test]
    fn the_console_map_pane() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("Rome");
        w.connected = true;
        app.worlds.push(w);
        app.current_world_index = 0;
        app.input_height = 1;
        for json in [
            r#"{"num":"F","name":"The Forum","area":"Forum","exits":{"e":"G"}}"#,
            r#"{"num":"G","name":"Via Sacra","area":"Forum","exits":{"w":"F","e":"H"}}"#,
            r#"{"num":"H","name":"Via Sacra, East","area":"Forum","exits":{"w":"G"}}"#,
        ] {
            app.map_on_gmcp(0, "Room.Info", json);
        }
        let screen = |app: &mut App| -> Vec<String> {
            let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
            terminal.draw(|f| crate::rendering::ui(f, app)).unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..30u16).map(|y| (0..40u16).map(|x| buf.get(x, y).symbol().to_string()).collect::<String>()).collect()
        };
        screen(&mut app);
        let closed_height = app.output_height;
        assert_eq!(app.map_rows, 0);
        app.console_map_command("");
        let rows = screen(&mut app);
        assert!(app.map_rows > 0);
        assert_eq!(app.output_height, closed_height - app.map_rows, "the output gave the pane its rows");
        assert_eq!(crate::rendering::console_panes(&app)[0].y, app.map_rows, "the pane below it starts after it");
        let bar = &rows[app.map_rows as usize - 1];
        assert!(bar.contains("Map · Forum · level 0 · 3 rooms"), "the bar: {bar:?}");
        let map: String = rows[..app.map_rows as usize - 1].join("\n");
        assert!(map.contains("[ ]-[ ]-[@]"), "the rooms, you at H: {map}");
        app.console_map_command("path the forum");
        assert_eq!(app.map_pane.as_ref().unwrap().route, ["G", "F"]);
        let map: String = screen(&mut app)[..app.map_rows as usize - 1].join("\n");
        assert!(map.contains("[X]-[ ]-[@]"), "the destination marked: {map}");
        app.console_map_command("");
        screen(&mut app);
        assert_eq!((app.map_rows, app.output_height), (0, closed_height));
    }

    /// An area named as the user typed it is found under the key the game gave it: the
    /// server's map by its rooms, an attached client's by the names the server sent.
    #[test]
    fn an_area_is_found_by_its_name() {
        let mut g = crate::map::model::MapGraph::default();
        g.insert(crate::map::model::Room {
            id: "1".into(),
            name: "On a hill".into(),
            area: "#45".into(),
            area_name: "Barren hills".into(),
            env: String::new(),
            x: 0,
            y: 0,
            z: 0,
            placed: crate::map::model::Placed::Given,
            given: None,
            exits: Vec::new(),
        });
        assert_eq!(g.area_key("#45").as_deref(), Some("#45"));
        assert_eq!(g.area_key("barren HILLS").as_deref(), Some("#45"));
        assert_eq!(g.area_key("Elsewhere"), None);
        let mut m = MirroredMap::default();
        assert_eq!(m.area_key("Barren hills"), "Barren hills", "nothing known yet: the name itself");
        m.note_area_name("#45", "Barren hills");
        m.note_area_name("#46", "");
        assert_eq!(m.area_key("barren hills"), "#45");
        assert_eq!(m.area_key("#45"), "#45");
        assert!(!m.names.contains_key("#46"), "a change's empty name isn't one");
    }

    #[test]
    fn map_command_forms() {
        assert_eq!(MapCmd::parse(""), MapCmd::Toggle);
        assert_eq!(MapCmd::parse("go  Palatine Steps "), MapCmd::Go("Palatine Steps".into()));
        assert_eq!(MapCmd::parse("LEVEL up"), MapCmd::Level("up".into()));
        assert_eq!(MapCmd::parse("area"), MapCmd::Area(None));
        assert_eq!(MapCmd::parse("frobnicate"), MapCmd::Help);
        assert!(MapCmd::parse("hide").is_pane() && !MapCmd::parse("stop").is_pane());
    }
}
