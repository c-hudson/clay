//! Clay's own requests to the Wayland compositor (Linux; user request 2026-11-16, for
//! KDE Plasma, whose session is Wayland).
//!
//! GPUI keeps its Wayland objects to itself but hands out the real `wl_display` and
//! each window's `wl_surface` (raw-window-handle). Clay attaches to that display as a
//! guest: libwayland gives a guest an event queue of its own, so GPUI's events stay
//! GPUI's and Clay's objects answer to Clay. What GPUI has no call for goes through here:
//!
//! - the window's opacity (`wp_alpha_modifier_v1`), for the settings' Transparency;
//! - asking for attention without taking the focus (`xdg_activation_v1` with a token
//!   that carries no input serial: the compositor refuses the activation and flags the
//!   window instead, which is what Qt's `QWindow::alert` does), for Taskbar Alert;
//! - where the window's menu is on the session bus (`org_kde_kwin_appmenu`), for
//!   Plasma's Global Menu.
//!
//! Every function is a no-op that answers `false` on an X11 window and where the
//! compositor lacks the protocol. Main thread only: the state is a thread-local, and
//! the requests are made from GPUI's own callbacks.
//!
//! Nothing here reads the socket on a timer. GPUI's loop reads it for every queue; the
//! one reply Clay waits for (an activation token) is fetched with a round trip on Clay's
//! queue, which leaves GPUI's events in GPUI's queue for its next turn.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1;
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_v1::WpAlphaModifierV1;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{self, XdgActivationTokenV1};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols_plasma::appmenu::client::org_kde_kwin_appmenu::OrgKdeKwinAppmenu;
use wayland_protocols_plasma::appmenu::client::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager;

/// The compositor's globals Clay uses, and what it made for each window.
struct State {
    alpha: Option<WpAlphaModifierV1>,
    activation: Option<XdgActivationV1>,
    appmenus: Option<OrgKdeKwinAppmenuManager>,
    /// By the `wl_surface`'s address.
    windows: HashMap<usize, PerWindow>,
}

struct PerWindow {
    /// GPUI's surface, as an argument for Clay's requests. Its liveness is GPUI's: the
    /// two share one wayland-backend, so `is_alive` turns false when GPUI destroys it.
    surface: WlSurface,
    alpha: Option<WpAlphaModifierSurfaceV1>,
    appmenu: Option<OrgKdeKwinAppmenu>,
}

impl PerWindow {
    /// Destructor requests only: legal after the surface itself is gone.
    fn release(self) {
        if let Some(alpha) = self.alpha {
            alpha.destroy();
        }
        if let Some(menu) = self.appmenu {
            menu.release();
        }
    }
}

struct Line {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

thread_local! {
    /// Never dropped: at exit the display is GPUI's to close, and a guest's queue
    /// destroyed after that would touch freed memory.
    static LINE: RefCell<Option<ManuallyDrop<Line>>> = const { RefCell::new(None) };
}

impl Line {
    fn open(display: *mut c_void) -> Option<Line> {
        // SAFETY: GPUI's own connected `wl_display`, taken from a live window just now;
        // GPUI keeps it for the life of the process.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let state = State {
            alpha: globals.bind(&qh, 1..=1, ()).ok(),
            activation: globals.bind(&qh, 1..=1, ()).ok(),
            appmenus: globals.bind(&qh, 1..=2, ()).ok(),
            windows: HashMap::new(),
        };
        Some(Line { conn, queue, state })
    }

    /// The entry for this surface, made at first use; a closed window's is released
    /// (its address may be the next window's).
    fn window(&mut self, surface: *mut c_void) -> Option<usize> {
        let dead: Vec<usize> = self.state.windows.iter().filter(|(_, w)| !w.surface.is_alive()).map(|(k, _)| *k).collect();
        for key in dead {
            if let Some(w) = self.state.windows.remove(&key) {
                w.release();
            }
        }
        let key = surface as usize;
        if !self.state.windows.contains_key(&key) {
            // SAFETY: the `wl_surface` of a window GPUI is showing right now.
            let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
            let surface = WlSurface::from_id(&self.conn, id).ok()?;
            self.state.windows.insert(key, PerWindow { surface, alpha: None, appmenu: None });
        }
        Some(key)
    }
}

/// The window's `wl_display` and `wl_surface`, when it is a Wayland window.
fn pointers(window: &gpui::Window) -> Option<(*mut c_void, *mut c_void)> {
    let RawWindowHandle::Wayland(w) = HasWindowHandle::window_handle(window).ok()?.as_raw() else { return None };
    let RawDisplayHandle::Wayland(d) = HasDisplayHandle::display_handle(window).ok()?.as_raw() else { return None };
    Some((d.display.as_ptr(), w.surface.as_ptr()))
}

/// Run `f` with Clay's line to the compositor and this window's entry. `None` on an X11
/// window, or when the display can't be attached to.
fn with<R>(window: &gpui::Window, f: impl FnOnce(&mut Line, usize) -> R) -> Option<R> {
    let (display, surface) = pointers(window)?;
    LINE.with(|line| {
        let mut line = line.try_borrow_mut().ok()?;
        if line.is_none() {
            *line = Line::open(display).map(ManuallyDrop::new);
        }
        let line: &mut Line = line.as_mut()?;
        let key = line.window(surface)?;
        let result = f(line, key);
        let _ = line.conn.flush();
        Some(result)
    })
}

/// Whether this is a Wayland window (the X11 routes don't apply to it).
pub fn is_wayland(window: &gpui::Window) -> bool {
    pointers(window).is_some()
}

/// The whole window at this opacity: `value` is a fraction of `u32::MAX`, as X11's
/// `_NET_WM_WINDOW_OPACITY` is. It takes effect with the surface's next commit, so the
/// caller draws a frame. `false` where it can't be done.
pub fn set_opacity(window: &gpui::Window, value: u32) -> bool {
    with(window, |line, key| {
        let qh = line.queue.handle();
        let Some(manager) = line.state.alpha.clone() else { return false };
        let Some(w) = line.state.windows.get_mut(&key) else { return false };
        let surface = w.surface.clone();
        w.alpha.get_or_insert_with(|| manager.get_surface(&surface, &qh, ())).set_multiplier(value);
        true
    })
    .unwrap_or(false)
}

/// Flag the window as wanting attention, without giving it the focus. `false` where it
/// can't be done.
pub fn request_attention(window: &gpui::Window) -> bool {
    with(window, |line, key| {
        let qh = line.queue.handle();
        let Some(activation) = line.state.activation.clone() else { return false };
        let Some(w) = line.state.windows.get(&key) else { return false };
        // No `set_serial`: a token without one is never good for the focus.
        let token = activation.get_activation_token(&qh, key);
        token.set_app_id("clay".into());
        token.set_surface(&w.surface);
        token.commit();
        // The token's string comes straight back; its handler (below) activates with it.
        line.queue.roundtrip(&mut line.state).is_ok()
    })
    .unwrap_or(false)
}

/// Whether the compositor takes a window's menu address (KWin does; Plasma's Global
/// Menu reads it).
pub fn has_appmenu(window: &gpui::Window) -> bool {
    with(window, |line, _| line.state.appmenus.is_some()).unwrap_or(false)
}

/// This window's menu is the `com.canonical.dbusmenu` object at `path` of the session
/// bus's `service`, for as long as the window lives (a closed window's entry is released
/// with it). `false` where it can't be said.
pub fn set_appmenu(window: &gpui::Window, service: &str, path: &str) -> bool {
    with(window, |line, key| {
        let qh = line.queue.handle();
        let Some(manager) = line.state.appmenus.clone() else { return false };
        let Some(w) = line.state.windows.get_mut(&key) else { return false };
        let surface = w.surface.clone();
        w.appmenu.get_or_insert_with(|| manager.create(&surface, &qh, ())).set_address(service.into(), path.into());
        true
    })
    .unwrap_or(false)
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationTokenV1, usize> for State {
    fn event(state: &mut Self, token: &XdgActivationTokenV1, event: xdg_activation_token_v1::Event, key: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_activation_token_v1::Event::Done { token: text } = event {
            if let (Some(activation), Some(w)) = (&state.activation, state.windows.get(key)) {
                activation.activate(text, &w.surface);
            }
            token.destroy();
        }
    }
}

delegate_noop!(State: ignore WpAlphaModifierV1);
delegate_noop!(State: ignore WpAlphaModifierSurfaceV1);
delegate_noop!(State: ignore XdgActivationV1);
delegate_noop!(State: ignore OrgKdeKwinAppmenuManager);
delegate_noop!(State: ignore OrgKdeKwinAppmenu);
