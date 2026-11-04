//! The map pane (src/map; app.js "Map pane" is its twin): beside the output, between it and
//! a right tab column. Drawn from what the server sends - `MapState` (where you are, the
//! walk) and `MapArea` (the rooms of the area's level shown), kept by the core
//! (`World::map`). A room clicked asks `MapRoute` for its route; Walk here asks the server to
//! walk there; Stop and Esc send `MapStop`. The pane tells the server what it shows with
//! `MapWatch` whenever that changes. The main window's: a `/window` follower has none.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::{canvas, div, point, prelude::*, px, Bounds, Context, Entity, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, ScrollWheelEvent, SharedString, Window};
use gpui_component::input::{Input, InputEvent, InputState};

use super::ClayView;
use super::kit::Btn;
use crate::map::wire::{MapRoomBrief, MapRoomMsg, MapStepMsg, MapViewReq};
use crate::websocket::WsMessage;

/// Each room drawn, where it was drawn (for a click), shared with the canvas's paint.
type Hits = Rc<RefCell<Vec<(Bounds<Pixels>, String)>>>;

/// Pixels per grid step at zoom 1, and a room's side.
const UNIT: f32 = 28.0;
const ROOM: f32 = 14.0;
/// The pane's width.
pub(super) const MAP_WIDTH: f32 = 340.0;

/// A room picked on the map: its route from where you are (`MapRouteResult`).
#[derive(Clone, Debug)]
pub(super) struct Picked {
    world: usize,
    id: String,
    /// Where you were when its route was worked out.
    from: Option<String>,
    room: Option<MapRoomMsg>,
    route: Vec<MapStepMsg>,
    error: Option<String>,
    req: u64,
}

pub(super) struct MapPane {
    pub open: bool,
    /// The area and level shown; `None` follows you.
    area: Option<String>,
    level: Option<i32>,
    zoom: f32,
    pan: (f32, f32),
    picked: Option<Picked>,
    req: u64,
    /// Each room drawn, where it was drawn: for a click.
    hits: Hits,
    drag: Option<(Point<Pixels>, (f32, f32))>,
    dragged: bool,
    search: Entity<InputState>,
    results: Vec<MapRoomBrief>,
}

impl MapPane {
    pub fn new(window: &mut Window, cx: &mut Context<ClayView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a room by name"));
        cx.subscribe_in(&search, window, |this, input, ev: &InputEvent, _w, cx| match ev {
            InputEvent::Change => {
                let q = input.read(cx).value().trim().to_string();
                if q.is_empty() {
                    this.map.results.clear();
                    cx.notify();
                    return;
                }
                this.map.req += 1;
                let msg = WsMessage::MapSearch { world_index: this.session.core.current, request_id: this.map.req, query: q };
                let fx = this.session.core.map_send(msg);
                this.after(fx, cx);
            }
            InputEvent::PressEnter { .. } => {
                if let Some(first) = this.map.results.first().cloned() {
                    this.map.results.clear();
                    if !first.area.is_empty() {
                        this.map.area = Some(first.area);
                        this.map.level = Some(first.level);
                        this.map.pan = (0.0, 0.0);
                    }
                    this.map_pick(first.id, cx);
                }
            }
            _ => {}
        })
        .detach();
        let open = crate::native_gui::core::store::pref("map_open").is_some_and(|v| v == "1");
        MapPane {
            open,
            area: None,
            level: None,
            zoom: 1.0,
            pan: (0.0, 0.0),
            picked: None,
            req: 0,
            hits: Rc::new(RefCell::new(Vec::new())),
            drag: None,
            dragged: false,
            search,
            results: Vec::new(),
        }
    }
}

impl ClayView {
    /// Open or close the pane (remembered for the next start).
    pub(super) fn map_set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.map.open = open;
        if !open {
            self.map.picked = None;
        }
        crate::native_gui::core::store::set_pref("map_open", if open { "1" } else { "0" });
        cx.notify();
    }

    /// `/map`'s pane forms, typed here or sent back by the server: true when it was one.
    pub(super) fn map_pane_command(&mut self, line: &str, cx: &mut Context<Self>) -> bool {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.first().is_some_and(|w| w.eq_ignore_ascii_case("/map") || w.eq_ignore_ascii_case("/@map")) {
            return false;
        }
        let verb = words.get(1).map(|w| w.to_ascii_lowercase());
        if !matches!(verb.as_deref(), None | Some("show" | "on" | "open" | "hide" | "off" | "close" | "center" | "centre" | "area" | "level")) {
            return false;
        }
        if self.is_follower() {
            self.client_line("The map is in the main window.", cx);
            return true;
        }
        let rest = words.get(2..).map(|r| r.join(" ")).unwrap_or_default();
        match verb.as_deref() {
            None => self.map_set_open(!self.map.open, cx),
            Some("show" | "on" | "open") => self.map_set_open(true, cx),
            Some("hide" | "off" | "close") => self.map_set_open(false, cx),
            Some("center" | "centre") => {
                self.map.area = None;
                self.map.level = None;
                self.map.pan = (0.0, 0.0);
                self.map_set_open(true, cx);
            }
            Some("area") => {
                self.map.area = (!rest.is_empty()).then_some(rest);
                self.map.level = None;
                self.map_set_open(true, cx);
            }
            Some("level") => {
                let cur = self.map_shown().map_or(0, |(_, l)| l);
                let level = match rest.as_str() {
                    "up" => cur + 1,
                    "down" => cur - 1,
                    n => match n.parse() {
                        Ok(n) => n,
                        Err(_) => return true,
                    },
                };
                self.map.area = self.map_shown().map(|(a, _)| a);
                self.map.level = Some(level);
                self.map_set_open(true, cx);
            }
            _ => return false,
        }
        true
    }

    /// The area and level shown for the current world.
    fn map_shown(&self) -> Option<(String, i32)> {
        let w = self.session.core.current_world()?;
        let st = w.map.state.as_ref();
        let here_area = st.filter(|s| s.here.is_some()).map(|s| s.area.clone());
        // A typed area name is found again under the key the server answers with.
        let area = self.map.area.as_deref().map(|a| w.map.area_key(a)).or(here_area.clone())?;
        let level = self.map.level.or_else(|| st.filter(|_| Some(&area) == here_area.as_ref()).map(|s| s.level)).or_else(|| w.map.levels.get(&area).and_then(|l| l.first().copied())).unwrap_or(0);
        Some((area, level))
    }

    /// `MapWatch` when what the pane shows changed (the core sends only a change, and
    /// again after a login).
    pub(super) fn map_watch_if_changed(&mut self, cx: &mut Context<Self>) {
        if self.is_follower() {
            return;
        }
        let views: Vec<MapViewReq> = if self.map.open {
            vec![MapViewReq { world_index: self.session.core.current, area: self.map.area.clone(), level: self.map.level }]
        } else {
            Vec::new()
        };
        let fx = self.session.core.map_watch(views);
        if !fx.is_empty() {
            self.after(fx, cx);
        }
    }

    /// A reply for the map: true when it was one.
    pub(super) fn map_on_reply(&mut self, msg: &WsMessage, cx: &mut Context<Self>) -> bool {
        match msg {
            WsMessage::MapRouteResult { request_id, room, route, error, .. } => {
                if let Some(p) = self.map.picked.as_mut().filter(|p| p.req == *request_id) {
                    p.room = room.clone();
                    p.route = route.clone();
                    p.error = error.clone();
                    cx.notify();
                }
                true
            }
            WsMessage::MapSearchResult { world_index, rooms, .. } => {
                if *world_index == self.session.core.current {
                    self.map.results = rooms.clone();
                    cx.notify();
                }
                true
            }
            _ => false,
        }
    }

    fn map_pick(&mut self, id: String, cx: &mut Context<Self>) {
        self.map.req += 1;
        let world = self.session.core.current;
        let from = self.map_here(world);
        self.map.picked = Some(Picked { world, id: id.clone(), from, room: None, route: Vec::new(), error: None, req: self.map.req });
        let fx = self.session.core.map_send(WsMessage::MapRoute { world_index: world, request_id: self.map.req, to: format!("#{id}"), walk: false });
        self.after(fx, cx);
        cx.notify();
    }

    /// The room you are in, in a world.
    fn map_here(&self, world: usize) -> Option<String> {
        self.session.core.worlds.get(world).and_then(|w| w.map.state.as_ref()).and_then(|s| s.here.as_ref()).map(|h| h.id.clone())
    }

    /// A picked room's route was worked out from where you were: once you are somewhere
    /// else (a walk over, or moved by hand) it is asked for again, or dropped when you
    /// are there (app.js `mapOnState`).
    pub(super) fn map_refresh_pick(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.map.picked.clone() else { return };
        let walking = self.session.core.worlds.get(p.world).and_then(|w| w.map.state.as_ref()).is_some_and(|s| s.walk.is_some());
        let here = self.map_here(p.world);
        if walking || here == p.from {
            return;
        }
        if here.as_deref() == Some(p.id.as_str()) {
            self.map.picked = None;
        } else if p.world == self.session.core.current {
            self.map_pick(p.id, cx);
        }
    }

    fn map_walk(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.map.picked.clone() else { return };
        self.map.req += 1;
        let fx = self.session.core.map_send(WsMessage::MapRoute { world_index: p.world, request_id: self.map.req, to: format!("#{}", p.id), walk: true });
        self.after(fx, cx);
    }

    /// Stop the walk in the world shown (Stop, Esc). True when there was one.
    pub(super) fn map_stop_walk(&mut self, cx: &mut Context<Self>) -> bool {
        let world = self.session.core.current;
        let walking = self.session.core.current_world().and_then(|w| w.map.state.as_ref()).is_some_and(|s| s.walk.is_some());
        if walking {
            let fx = self.session.core.map_send(WsMessage::MapStop { world_index: world });
            self.after(fx, cx);
        }
        walking
    }

    /// The pane, when it is open (the main window's only).
    pub(super) fn map_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.map.open || self.is_follower() {
            return None;
        }
        let _ = window;
        let t = self.chrome.tokens.clone();
        // The canvas and the level box sit on the output's own background (style.css
        // `.map-canvas`), the pane on the surface colour.
        let ground = super::hsla(self.palette.bg);
        let ui = self.fonts.ui.clone();
        let mono = self.fonts.primary.clone();
        let world = self.session.core.current_world();
        let name = world.map_or(String::new(), |w| w.name.clone());
        let state = world.and_then(|w| w.map.state.clone());
        let shown = self.map_shown();
        let rooms: Vec<MapRoomMsg> = match (&shown, world) {
            (Some((area, level)), Some(w)) => w.map.areas.get(&(area.clone(), *level)).cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        let here = state.as_ref().and_then(|s| s.here.as_ref()).map(|h| h.id.clone());
        let walk = state.as_ref().and_then(|s| s.walk.clone());
        let picked = self.map.picked.clone().filter(|p| p.world == self.session.core.current);
        let target = walk.as_ref().map(|w| w.to.clone()).or(picked.as_ref().map(|p| p.id.clone()));
        let mut route: HashSet<String> = here.iter().cloned().collect();
        match (&walk, &picked) {
            (Some(w), _) => route.extend(w.steps.iter().skip(w.next as usize).map(|s| s.to.clone())),
            (None, Some(p)) => route.extend(p.route.iter().map(|s| s.to.clone())),
            _ => {}
        }
        let area_name = match (&shown, &state) {
            (Some((area, _)), Some(s)) if &s.area == area && !s.area_name.is_empty() => s.area_name.clone(),
            (Some((area, _)), _) => world.and_then(|w| w.map.names.get(area)).cloned().unwrap_or_else(|| area.clone()),
            _ => String::new(),
        };
        let level = shown.as_ref().map_or(0, |(_, l)| *l);
        let icon_btn = |id: &'static str, glyph: &'static str, label: &'static str| {
            let hover = t.hover;
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .w(px(26.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .text_color(t.fg2)
                .hover(move |s| s.bg(hover))
                .child(glyph)
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .px(px(10.0))
            .py(px(7.0))
            .border_b_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(10.0)).text_color(t.muted).whitespace_nowrap().overflow_hidden().text_ellipsis().child(format!("MAP · {}", name.to_uppercase())))
                    .child(div().text_size(px(14.0)).font_weight(gpui::FontWeight::BOLD).text_color(t.fg).whitespace_nowrap().overflow_hidden().text_ellipsis().child(area_name)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .p(px(1.0))
                    .border_1()
                    .border_color(t.border_subtle)
                    .rounded(px(6.0))
                    .bg(ground)
                    .child(icon_btn("map-level-down", "▾", "Level down").on_click(cx.listener(|this, _, _w, cx| {
                        this.map_pane_command("/map level down", cx);
                    })))
                    .child(div().text_size(px(11.5)).text_color(t.fg2).px(px(4.0)).whitespace_nowrap().child(format!("Level {level}")))
                    .child(icon_btn("map-level-up", "▴", "Level up").on_click(cx.listener(|this, _, _w, cx| {
                        this.map_pane_command("/map level up", cx);
                    }))),
            )
            .child(icon_btn("map-zoom-out", "−", "Zoom out").on_click(cx.listener(|this, _, _w, cx| {
                this.map.zoom = (this.map.zoom / 1.25).max(0.3);
                cx.notify();
            })))
            .child(icon_btn("map-zoom-in", "+", "Zoom in").on_click(cx.listener(|this, _, _w, cx| {
                this.map.zoom = (this.map.zoom * 1.25).min(3.0);
                cx.notify();
            })))
            .child(icon_btn("map-center", "◎", "Centre on my room").on_click(cx.listener(|this, _, _w, cx| {
                this.map.picked = None;
                this.map_pane_command("/map center", cx);
            })))
            .child(icon_btn("map-close", "✕", "Close map").on_click(cx.listener(|this, _, _w, cx| this.map_set_open(false, cx))));
        let mut search = div().relative().px(px(10.0)).py(px(7.0)).child(
            Input::new(&self.map.search).w_full().text_color(t.fg).font_family(ui.clone()).text_size(px(12.5)).bg(ground).rounded(px(6.0)),
        );
        if !self.map.results.is_empty() {
            let mut list = div().absolute().top(px(40.0)).left(px(10.0)).right(px(10.0)).bg(t.elevated).border_1().border_color(t.border).rounded(px(5.0)).occlude().flex().flex_col();
            for (i, r) in self.map.results.iter().take(12).enumerate() {
                let id = r.id.clone();
                let place = (!r.area.is_empty()).then(|| (r.area.clone(), r.level));
                let hover = t.hover;
                list = list.child(
                    div()
                        .id(("map-result", i))
                        .role(Role::Button)
                        .aria_label(SharedString::from(r.name.clone()))
                        .px(px(8.0))
                        .py(px(5.0))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .child(div().text_color(t.fg).child(r.name.clone()))
                        .child(div().text_size(px(10.5)).text_color(t.muted).child(format!("{} · level {}", r.area_name, r.level)))
                        .on_click(cx.listener(move |this, _, _w, cx| {
                            this.map.results.clear();
                            // The room's own area and level (an older server: yours).
                            this.map.area = place.as_ref().map(|p| p.0.clone());
                            this.map.level = place.as_ref().map(|p| p.1);
                            this.map.pan = (0.0, 0.0);
                            this.map_pick(id.clone(), cx);
                        })),
                );
            }
            // Painted after the canvas below it, which would otherwise cover it.
            search = search.child(gpui::deferred(list).with_priority(1));
        }
        let colours = MapColours { room: t.elevated, room_edge: t.border, link: t.border, stub: t.dim, away: t.accent, you: t.accent, target: t.highlight, route: t.highlight, ground };
        let scene = Scene { rooms: rooms.clone(), here: here.clone(), target: target.clone(), route, zoom: self.map.zoom, pan: self.map.pan, colours, hits: self.map.hits.clone() };
        let empty = rooms.is_empty();
        let canvas_el = div()
            .id("map-canvas")
            .role(Role::Image)
            .aria_label(SharedString::from(format!("Map of {}{}", shown.as_ref().map_or("nothing yet", |(a, _)| a.as_str()), here.as_ref().and_then(|h| rooms.iter().find(|r| &r.id == h)).map_or(String::new(), |r| format!(": you are in {}", r.name)))))
            .relative()
            .flex_1()
            .min_h(px(140.0))
            .mx(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border_subtle)
            .overflow_hidden()
            .bg(ground)
            .child(canvas(move |_, _, _| scene, |bounds, scene, window, _cx| paint_map(bounds, scene, window)).size_full())
            .when(empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(px(14.0))
                        .text_color(t.muted)
                        .text_size(px(12.0))
                        .child(div().w_full().text_center().child(if self.session.core.server_map {
                            "No map yet: Clay builds it from the rooms the game reports over GMCP (Room.Info) as you move."
                        } else {
                            "The server this window is connected to has no map."
                        })),
                )
            })
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _w, _cx| {
                this.map.drag = Some((ev.position, this.map.pan));
                this.map.dragged = false;
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| {
                let Some((start, pan)) = this.map.drag else { return };
                if ev.pressed_button != Some(MouseButton::Left) {
                    this.map.drag = None;
                    return;
                }
                let (dx, dy) = (f32::from(ev.position.x - start.x), f32::from(ev.position.y - start.y));
                if dx.abs() + dy.abs() > 4.0 {
                    this.map.dragged = true;
                }
                if this.map.dragged {
                    this.map.pan = (pan.0 + dx, pan.1 + dy);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _w, cx| {
                this.map.drag = None;
                if this.map.dragged {
                    return;
                }
                let hit = this.map.hits.borrow().iter().find(|(b, _)| b.contains(&ev.position)).map(|(_, id)| id.clone());
                if let Some(id) = hit {
                    this.map_pick(id, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _w, cx| {
                let dy = f32::from(ev.delta.pixel_delta(px(16.0)).y);
                if dy != 0.0 {
                    this.map.zoom = (this.map.zoom * if dy > 0.0 { 1.15 } else { 1.0 / 1.15 }).clamp(0.3, 3.0);
                    cx.notify();
                }
                cx.stop_propagation();
            }));
        // What the colours mean (style.css `.map-legend`).
        let key = |swatch: gpui::Div, label: &'static str| div().flex().items_center().gap(px(5.0)).child(swatch).child(label);
        let legend = div()
            .flex()
            .flex_wrap()
            .gap_x(px(12.0))
            .gap_y(px(4.0))
            .px(px(12.0))
            .pt(px(6.0))
            .text_size(px(11.0))
            .text_color(t.muted)
            .child(key(div().w(px(10.0)).h(px(10.0)).rounded(px(3.0)).bg(t.accent), "You"))
            .child(key(div().w(px(10.0)).h(px(10.0)).rounded(px(3.0)).bg(t.highlight), "Destination"))
            .child(key(div().w(px(14.0)).h(px(3.0)).bg(t.highlight), "Route"))
            .child(key(div().w(px(14.0)).h(px(0.0)).border_t_2().border_dashed().border_color(t.dim), "Unexplored"));
        // The card: the room picked (else yours), its exits, the route or the walk, and
        // Walk here / Stop.
        let card_room = match &picked {
            Some(p) => p.room.clone(),
            None => state.as_ref().and_then(|s| s.here.clone()),
        };
        let route_text: Option<String> = if let Some(w) = &walk {
            Some(format!("Walking to {} · step {} of {}", w.to_name, w.next + 1, w.steps.len()))
        } else if let Some(p) = &picked {
            if let Some(e) = &p.error {
                Some(e.clone())
            } else if !p.route.is_empty() {
                Some(format!("{} · {} step{}", p.route.iter().map(|s| s.cmd.as_str()).collect::<Vec<_>>().join(", "), p.route.len(), if p.route.len() == 1 { "" } else { "s" }))
            } else if Some(&p.id) == here.as_ref() {
                Some("You are here.".to_string())
            } else {
                None
            }
        } else {
            state.as_ref().and_then(|s| s.last_stop.clone())
        };
        let can_walk = walk.is_none() && picked.as_ref().is_some_and(|p| p.error.is_none() && !p.route.is_empty());
        let kit = self.kit();
        let mut card = div()
            .id("map-card")
            .m(px(10.0))
            .p(px(11.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(t.border)
            .bg(t.elevated);
        if let Some(r) = &card_room {
            card = card
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(div().text_size(px(14.0)).font_weight(gpui::FontWeight::BOLD).text_color(t.fg).whitespace_nowrap().overflow_hidden().text_ellipsis().child(r.name.clone()))
                        .child(div().text_size(px(11.0)).text_color(t.muted).whitespace_nowrap().child(format!("#{} · level {}", r.id, r.z))),
                )
                .child(div().flex().flex_wrap().gap(px(4.0)).children(r.exits.iter().map(|e| {
                    let chip = div().px(px(6.0)).py(px(1.0)).rounded(px(4.0)).border_1().font_family(mono.clone()).text_size(px(11.0)).child(e.cmd.clone());
                    if e.kind == 3 { chip.border_dashed().border_color(t.dim).text_color(t.muted) } else { chip.border_color(t.border).bg(ground).text_color(t.fg) }
                })));
        }
        if let Some(text) = route_text {
            card = card.child(div().text_size(px(12.0)).text_color(if can_walk { t.highlight } else { t.fg2 }).child(text));
        }
        if can_walk {
            card = card.child(div().flex().child(kit.button("map-walk", "Walk here", Btn::Primary, Some(Box::new(cx.listener(|this, _, _w, cx| this.map_walk(cx))))).role(Role::Button).aria_label("Walk here")));
        }
        if walk.is_some() {
            card = card.child(div().flex().child(
                kit.button(
                    "map-stop",
                    "Stop",
                    Btn::Danger,
                    Some(Box::new(cx.listener(|this, _, _w, cx| {
                        this.map_stop_walk(cx);
                    }))),
                )
                .role(Role::Button)
                .aria_label("Stop the walk"),
            ));
        }
        let show_card = card_room.is_some() || walk.is_some();
        Some(
            div()
                .id("map-pane")
                .role(Role::Pane)
                .aria_label("Map")
                .w(px(MAP_WIDTH))
                .flex_shrink_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(t.surface)
                .border_l_1()
                .border_color(t.border_subtle)
                .font_family(ui)
                .text_size(px(12.0))
                .child(header)
                .child(search)
                .child(canvas_el)
                .child(legend)
                .when(show_card, |d| d.child(card))
                .into_any_element(),
        )
    }
}

#[derive(Clone)]
struct MapColours {
    room: Hsla,
    room_edge: Hsla,
    link: Hsla,
    stub: Hsla,
    away: Hsla,
    you: Hsla,
    target: Hsla,
    route: Hsla,
    ground: Hsla,
}

/// What the canvas draws.
struct Scene {
    rooms: Vec<MapRoomMsg>,
    here: Option<String>,
    target: Option<String>,
    route: HashSet<String>,
    zoom: f32,
    pan: (f32, f32),
    colours: MapColours,
    hits: Hits,
}

/// A direction's step on the grid.
fn dir_step(dir: &str) -> Option<(f32, f32)> {
    Some(match dir {
        "n" => (0.0, -1.0),
        "ne" => (1.0, -1.0),
        "e" => (1.0, 0.0),
        "se" => (1.0, 1.0),
        "s" => (0.0, 1.0),
        "sw" => (-1.0, 1.0),
        "w" => (-1.0, 0.0),
        "nw" => (-1.0, -1.0),
        _ => return None,
    })
}

/// Rooms as squares on a grid, exits as lines between them (dashed where unexplored), the
/// route in the highlight, you in the accent; centred on you, else on the room picked or
/// walked to, else on the rooms' middle; then the pan and zoom.
fn paint_map(bounds: Bounds<Pixels>, s: Scene, window: &mut Window) {
    let c = &s.colours;
    window.paint_quad(gpui::fill(bounds, c.ground));
    s.hits.borrow_mut().clear();
    if s.rooms.is_empty() {
        return;
    }
    let by_id: HashMap<&str, &MapRoomMsg> = s.rooms.iter().map(|r| (r.id.as_str(), r)).collect();
    let centre = s.here.as_deref().and_then(|h| by_id.get(h)).or_else(|| s.target.as_deref().and_then(|t| by_id.get(t)));
    let (cx0, cy0) = match centre {
        Some(r) => (r.x as f32, r.y as f32),
        None => {
            let xs = s.rooms.iter().map(|r| r.x);
            let ys = s.rooms.iter().map(|r| r.y);
            ((xs.clone().min().unwrap_or(0) + xs.max().unwrap_or(0)) as f32 / 2.0, (ys.clone().min().unwrap_or(0) + ys.max().unwrap_or(0)) as f32 / 2.0)
        }
    };
    let unit = UNIT * s.zoom;
    let mid = point(bounds.origin.x + bounds.size.width / 2.0 + px(s.pan.0), bounds.origin.y + bounds.size.height / 2.0 + px(s.pan.1));
    let at = |r: &MapRoomMsg| point(mid.x + px((r.x as f32 - cx0) * unit), mid.y + px((r.y as f32 - cy0) * unit));
    let line = |window: &mut Window, a: Point<Pixels>, b: Point<Pixels>, width: f32, colour: Hsla, dashed: bool| {
        let mut p = PathBuilder::stroke(px(width));
        if dashed {
            p = p.dash_array(&[px(3.0), px(3.0)]);
        }
        p.move_to(a);
        p.line_to(b);
        if let Ok(path) = p.build() {
            window.paint_path(path, colour);
        }
    };
    let mut drawn: HashSet<(String, String)> = HashSet::new();
    for r in &s.rooms {
        let a = at(r);
        for e in &r.exits {
            match (e.kind, e.to.as_deref().and_then(|t| by_id.get(t))) {
                (0, Some(other)) => {
                    let key = if r.id < other.id { (r.id.clone(), other.id.clone()) } else { (other.id.clone(), r.id.clone()) };
                    if !drawn.insert(key) {
                        continue;
                    }
                    let on_route = s.route.contains(&r.id) && s.route.contains(&other.id);
                    line(window, a, at(other), if on_route { 4.0 } else { 2.0 }, if on_route { c.route } else { c.link }, false);
                }
                (2 | 3, _) => {
                    if let Some((dx, dy)) = e.dir.as_deref().and_then(dir_step) {
                        let b = point(a.x + px(dx * unit * 0.5), a.y + px(dy * unit * 0.5));
                        line(window, a, b, 2.0, if e.kind == 3 { c.stub } else { c.away }, e.kind == 3);
                    }
                }
                _ => {}
            }
        }
    }
    let side = (ROOM * s.zoom.max(0.6)).max(8.0);
    for r in &s.rooms {
        let p = at(r);
        let b = Bounds::new(point(p.x - px(side / 2.0), p.y - px(side / 2.0)), gpui::size(px(side), px(side)));
        let here = s.here.as_deref() == Some(r.id.as_str());
        let target = !here && s.target.as_deref() == Some(r.id.as_str());
        let (fill, edge) = if here {
            (c.you, c.you)
        } else if target {
            (c.target, c.target)
        } else if s.route.contains(&r.id) {
            (c.room, c.route)
        } else {
            (c.room, c.room_edge)
        };
        window.paint_quad(gpui::quad(b, px(3.0), fill, px(2.0), edge, gpui::BorderStyle::Solid));
        // A way up or down: a small triangle in the room.
        let up = r.exits.iter().any(|e| e.dir.as_deref() == Some("u"));
        let down = r.exits.iter().any(|e| e.dir.as_deref() == Some("d"));
        if up || down {
            let mark = if here || target { c.ground } else { c.link };
            let h = side * 0.22;
            let mut tri = PathBuilder::fill();
            let (tip, base) = if up { (p.y - px(h), p.y + px(h)) } else { (p.y + px(h), p.y - px(h)) };
            tri.move_to(point(p.x, tip));
            tri.line_to(point(p.x + px(h), base));
            tri.line_to(point(p.x - px(h), base));
            tri.close();
            if let Ok(path) = tri.build() {
                window.paint_path(path, mark);
            }
        }
        s.hits.borrow_mut().push((b, r.id.clone()));
    }
}
