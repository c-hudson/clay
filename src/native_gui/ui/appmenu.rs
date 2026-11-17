//! The hamburger menu as the desktop's own app menu (user request 2026-10-31): on Linux the
//! menu a panel applet shows for the active window (xfce4-appmenu-plugin, MATE's and
//! Budgie's - all vala-panel-appmenu), on macOS the menu bar. Nothing is drawn in the
//! window: on Linux the menu is offered over D-Bus and the window only says where it is,
//! so without an applet nobody reads it. The WebView GUI has none (user ruling).
//!
//! It is the hamburger's list (chrome.rs `MAIN_MENU`) and rule (`menu_shows`), then Quit,
//! and a choice runs `ClayView::menu_action`, as a click in the hamburger does.
//!
//! On Linux it is GTK's own protocol, which the applet reads off the window
//! (vala-panel-appmenu `appmenu-wnck.vala`): `_GTK_UNIQUE_BUS_NAME` and
//! `_GTK_APP_MENU_OBJECT_PATH` name an `org.gtk.Menus` model, shown under the program's
//! name in place of the applet's own New/Quit, and `_GTK_WINDOW_OBJECT_PATH` names the
//! window's `org.gtk.Actions` (`win.`). KDE Plasma's Global Menu reads the same properties
//! through its `gmenu-dbusmenu-proxy`, which takes only a window typed as an ordinary one
//! (`_NET_WM_WINDOW_TYPE_NORMAL`, which GPUI leaves unset, so Clay sets it). Each Clay
//! window (the main one and its `/window`s) has its own actions, so a choice acts on the
//! window it was made for.
//!
//! On Wayland there is no window property to set. KDE has a protocol of its own for it
//! (user request 2026-11-16): the window tells KWin the bus name and path of a
//! `com.canonical.dbusmenu` object (`org_kde_kwin_appmenu`, wl.rs), which Plasma's Global
//! Menu and KWin's title-bar menu button read. So there each window has such an object,
//! with the same list under one **Clay** menu (as on macOS and under xfce's applet) and
//! the same way back to its window. Another Wayland compositor is offered nothing.
//!
//! The same list is the macOS Dock icon's menu (less Quit, which the Dock has), and on
//! Windows it heads each Clay window's own menu (a right-click on the title bar, or
//! Shift+right-click on its taskbar button; not Alt+Space, which is Clay's
//! `collapse_spaces`), less Quit, since that menu has Close.

use gpui::{Context, Window};

use super::ClayView;

/// What the menu's items depend on: the server's (Logout only on a multiuser server,
/// Change Password only where the server answers it, Import Picture only where it uploads
/// pictures).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Shown {
    pub multiuser: bool,
    pub picture_upload: bool,
    pub change_password: bool,
}

impl Default for Shown {
    /// A single-user Clay before its InitialState: everything a single user has.
    fn default() -> Self {
        Shown { multiuser: false, picture_upload: false, change_password: true }
    }
}

impl Shown {
    /// What the menus depend on, from a connection's server.
    pub(super) fn of(core: &crate::native_gui::core::client::ClientCore) -> Shown {
        Shown { multiuser: core.is_multiuser(), picture_upload: core.server_picture_upload, change_password: core.server_change_password }
    }

    /// Every combination a server can be.
    pub(super) fn every() -> Vec<Shown> {
        let mut all = Vec::new();
        for multiuser in [false, true] {
            for picture_upload in [false, true] {
                for change_password in [false, true] {
                    all.push(Shown { multiuser, picture_upload, change_password });
                }
            }
        }
        all
    }
}

/// A window's app menu, kept up to date as it draws (`ClayView::render`).
#[derive(Default)]
pub struct AppMenu {
    last: Option<Shown>,
    /// Linux: the window's id once its menu is offered; None until then, and for good
    /// where nothing reads one (`tried`: Wayland without KDE's protocol).
    #[cfg(target_os = "linux")]
    offered: Option<u64>,
    /// Windows: the window, and how many entries of ours head its menu.
    #[cfg(windows)]
    offered: Option<(isize, usize)>,
    #[cfg(any(target_os = "linux", windows))]
    tried: bool,
}

impl AppMenu {
    /// `owns_connection`: the window with the connection, whose server the menu bar
    /// follows on macOS (its `/window`s share the server).
    pub(super) fn apply(&mut self, window: &Window, shown: Shown, owns_connection: bool, cx: &mut Context<ClayView>) {
        // Each platform uses some of these.
        let _ = (window, owns_connection, &cx);
        #[cfg(target_os = "linux")]
        if !self.tried {
            self.tried = true;
            self.offered = linux::offer(window, cx);
        }
        #[cfg(windows)]
        if !self.tried {
            self.tried = true;
            self.offered = win::offer(window, cx).map(|hwnd| (hwnd, 0));
        }
        if self.last == Some(shown) {
            return;
        }
        self.last = Some(shown);
        #[cfg(target_os = "linux")]
        if let Some(id) = self.offered {
            linux::show(id, shown);
        }
        #[cfg(windows)]
        if let Some((hwnd, inserted)) = &mut self.offered {
            win::show(*hwnd, inserted, shown);
        }
        #[cfg(target_os = "macos")]
        if owns_connection {
            mac::set_menus(shown, cx);
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for AppMenu {
    fn drop(&mut self) {
        if let Some(id) = self.offered.take() {
            linux::withdraw(id);
        }
    }
}

/// The root element's half of the macOS menu bar: a choice made while this window is the
/// active one acts on it.
pub(super) fn listen(el: gpui::Div, cx: &mut Context<ClayView>) -> gpui::Div {
    #[cfg(target_os = "macos")]
    {
        use gpui::InteractiveElement as _;
        el.on_action(cx.listener(|this, choice: &mac::MenuCommand, window, cx| this.menu_action(choice.0, window, cx)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        el
    }
}

/// A choice made in a desktop menu, delivered to `window`'s `menu_action`.
#[cfg(any(target_os = "linux", windows))]
fn deliver(window: &Window, cx: &mut Context<ClayView>) -> futures::channel::mpsc::UnboundedSender<String> {
    use futures::StreamExt as _;
    let handle = window.window_handle();
    let (choices, mut chosen) = futures::channel::mpsc::unbounded::<String>();
    // By this window's handle, never `update_in`: that finds the window an entity was
    // last drawn in, and a `/window` draws reading its leader's view - a choice in the
    // main window's menu then went to the follower's window, and none came through
    // once that follower closed.
    cx.spawn(async move |view, cx| {
        while let Some(action) = chosen.next().await {
            let done = handle.update(cx, |_, window, cx| view.update(cx, |v, cx| v.menu_action(&action, window, cx)));
            if !matches!(done, Ok(Ok(()))) {
                break;
            }
        }
    })
    .detach();
    choices
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
mod items {
    use super::super::chrome::{MenuEntry, every_menu_section, menu_sections};
    use super::Shown;

    /// The app menu's last item, after the hamburger's.
    #[cfg_attr(windows, allow(dead_code))]
    pub const QUIT: MenuEntry = ("quit", "Quit", "");

    /// The sections as they show for `shown`, then Quit (the macOS menu bar's).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn sections(shown: Shown) -> Vec<Vec<MenuEntry>> {
        let mut sections = without_quit(shown);
        sections.push(vec![QUIT]);
        sections
    }

    /// The hamburger's sections alone: the Dock's menu and the Windows window menu, which
    /// have a Quit or a Close of their own.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn without_quit(shown: Shown) -> Vec<Vec<MenuEntry>> {
        menu_sections(shown)
    }

    /// Every item that can be in the Windows window menu, whatever the server.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    fn every() -> Vec<&'static str> {
        every_menu_section().into_iter().flatten().map(|(action, _, _)| action).collect()
    }

    /// The Windows menu's command id for `action`: a multiple of 16 (Windows uses a system
    /// command's low four bits itself), below 0xF000 where the system's own begin, and the
    /// same whichever items the server shows.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn command_id(action: &str) -> Option<usize> {
        every().iter().position(|a| *a == action).map(|i| 0x1000 + 16 * i)
    }

    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn action_of(id: usize) -> Option<&'static str> {
        every().into_iter().enumerate().find(|(i, _)| 0x1000 + 16 * i == id).map(|(_, a)| a)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_dock_and_window_menus_are_the_hamburgers_without_quit() {
            let all = without_quit(Shown { multiuser: true, picture_upload: true, change_password: false });
            assert!(all.iter().flatten().all(|(a, _, _)| *a != "quit"));
            assert_eq!(sections(Shown::default()).last().map(|s| s[0].0), Some("quit"), "the app menu ends with Quit");
            let single = without_quit(Shown::default());
            assert!(single.iter().flatten().all(|(a, _, _)| !matches!(*a, "logout" | "change-password" | "import-picture")));
            assert!(single.iter().flatten().any(|(a, _, _)| *a == "edit-world"));
            assert!(all.iter().flatten().any(|(a, _, _)| *a == "edit-world"), "a clay_multi user's own worlds");
            assert!(all.iter().flatten().all(|(a, _, _)| *a != "change-password"), "password.dat is the operator's");
            assert!(command_id("edit-world").is_some(), "the Windows menu has an id for it whatever the server");
        }

        #[test]
        fn window_menu_ids_are_the_systems_kind_and_round_trip() {
            let ids: Vec<usize> = every().iter().map(|a| command_id(a).expect("every item has one")).collect();
            assert!(ids.iter().all(|id| id % 16 == 0 && *id < 0xF000), "{ids:x?}");
            assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len(), "distinct");
            for a in every() {
                assert_eq!(action_of(command_id(a).unwrap()), Some(a));
            }
            assert_eq!(command_id("quit"), None);
            assert_eq!(action_of(0xF060), None, "SC_CLOSE stays the system's");
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) mod mac {
    use gpui::{AnyWindowHandle, App, Menu, MenuItem, WeakEntity};

    use super::super::ClayView;
    use super::{Shown, items};

    /// A menu bar choice: the hamburger item's action.
    #[derive(Clone, PartialEq, gpui::Action)]
    #[action(namespace = clay_menu, no_json)]
    pub struct MenuCommand(pub &'static str);

    /// The menu bar (its first menu is macOS's app menu), and the Dock icon's menu: the
    /// same less Quit, which the Dock adds itself. A Dock choice goes the menu bar's way
    /// (GPUI's `handle_menu_item`), so `listen` and `install` handle both.
    pub fn set_menus(shown: Shown, cx: &App) {
        cx.set_menus([Menu::new("Clay").items(entries(items::sections(shown)))]);
        cx.set_dock_menu(entries(items::without_quit(shown)));
    }

    fn entries(sections: Vec<Vec<super::super::chrome::MenuEntry>>) -> Vec<MenuItem> {
        let mut entries = Vec::new();
        for (i, section) in sections.into_iter().enumerate() {
            if i > 0 {
                entries.push(MenuItem::separator());
            }
            entries.extend(section.into_iter().map(|(action, label, _)| MenuItem::action(label, MenuCommand(action))));
        }
        entries
    }

    /// The menu bar from the start; a choice made while another window is active (Settings,
    /// Help) acts on the main window.
    pub fn install(main: WeakEntity<ClayView>, window: AnyWindowHandle, cx: &mut App) {
        cx.on_action(move |choice: &MenuCommand, cx| {
            let _ = window.update(cx, |_, w, cx| main.update(cx, |v, cx| v.menu_action(choice.0, w, cx)));
        });
        set_menus(Shown::default(), cx);
    }
}

#[cfg(windows)]
mod win {
    use gpui::{Context, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{DeleteMenu, GetSystemMenu, InsertMenuW, MF_BYPOSITION, MF_SEPARATOR, MF_STRING, WM_NCDESTROY, WM_SYSCOMMAND};
    use windows::core::{HSTRING, PCWSTR};

    use super::super::ClayView;
    use super::{Shown, items};

    /// Our subclass among the window's.
    const SUBCLASS: usize = 0xC1A7;

    type Choices = futures::channel::mpsc::UnboundedSender<String>;

    /// Watch this window's menu for our items' commands. None where it isn't a Win32
    /// window or can't be watched.
    pub(super) fn offer(window: &Window, cx: &mut Context<ClayView>) -> Option<isize> {
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => return None,
        };
        let choices = Box::into_raw(Box::new(super::deliver(window, cx)));
        // SAFETY: the box is freed by the subclass at WM_NCDESTROY, or here if it never
        // got it.
        let watched = unsafe { SetWindowSubclass(HWND(hwnd as *mut core::ffi::c_void), Some(subclass), SUBCLASS, choices as usize) };
        if !watched.as_bool() {
            drop(unsafe { Box::from_raw(choices) });
            return None;
        }
        Some(hwnd)
    }

    /// The menu's head as `shown` has it: ours (`inserted` of them) out, then in again.
    pub(super) fn show(hwnd: isize, inserted: &mut usize, shown: Shown) {
        // SAFETY: the window is this view's own and still open while it draws.
        let menu = unsafe { GetSystemMenu(HWND(hwnd as *mut core::ffi::c_void), false) };
        if menu.is_invalid() {
            return;
        }
        for _ in 0..*inserted {
            let _ = unsafe { DeleteMenu(menu, 0, MF_BYPOSITION) };
        }
        *inserted = 0;
        let mut entries = Vec::new();
        for section in items::without_quit(shown) {
            entries.extend(section.into_iter().map(Some));
            // Between sections, and before the system's own items.
            entries.push(None);
        }
        for (pos, entry) in entries.into_iter().enumerate() {
            let done = match entry {
                Some((action, label, shortcut)) => {
                    let Some(id) = items::command_id(action) else { continue };
                    // A key goes after a tab (right-aligned); a command (/worlds) is no key.
                    let text = if shortcut.is_empty() || shortcut.starts_with('/') { label.to_string() } else { format!("{label}\t{shortcut}") };
                    unsafe { InsertMenuW(menu, pos as u32, MF_BYPOSITION | MF_STRING, id, &HSTRING::from(text)) }
                }
                None => unsafe { InsertMenuW(menu, pos as u32, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null()) },
            };
            if done.is_err() {
                break;
            }
            *inserted += 1;
        }
    }

    /// Runs inside the window's menu loop: it only passes the choice on, and touches
    /// nothing of GPUI's.
    unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, id: usize, data: usize) -> LRESULT {
        if msg == WM_SYSCOMMAND {
            if let Some(action) = items::action_of(wparam.0 & 0xFFF0) {
                // SAFETY: `data` is the box `offer` made, alive until WM_NCDESTROY.
                let choices = unsafe { &*(data as *const Choices) };
                let _ = choices.unbounded_send(action.to_string());
                return LRESULT(0);
            }
        }
        if msg == WM_NCDESTROY {
            // SAFETY: the window's last message: the box is no longer anyone's.
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass), id) };
            drop(unsafe { Box::from_raw(data as *mut Choices) });
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::mpsc as std_mpsc;

    use futures::channel::mpsc;
    use gpui::{Context, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{OwnedValue, Signature, Str, Structure, Value};

    use super::super::ClayView;
    use super::super::chrome::{MenuEntry, every_menu_section, menu_always_shows, menu_shows};
    use super::{Shown, items};

    pub(super) const APP_MENU_PATH: &str = "/org/clay/Clay/menus/appmenu";

    impl Shown {
        fn shows(self, action: &str) -> bool {
            action == items::QUIT.0 || menu_shows(action, self)
        }
    }

    /// Every item, the ones a server lacks included, in its section: the model holds them
    /// all, and an item's action being disabled hides it.
    fn every() -> Vec<Vec<MenuEntry>> {
        let mut sections = every_menu_section();
        sections.push(vec![items::QUIT]);
        sections
    }

    fn window_path(id: u64) -> String {
        format!("/org/clay/Clay/window/{id}")
    }

    fn menu_path(id: u64) -> String {
        format!("/org/clay/Clay/dbusmenu/{id}")
    }

    /// What the windows ask of the D-Bus side, in order, on one thread.
    enum Job {
        /// An X11 window: GTK's model and actions, named by properties on the window.
        Offer { id: u64, xid: u32, choices: mpsc::UnboundedSender<String> },
        /// A Wayland window under KWin: a dbusmenu object; `ready` gets the bus name and
        /// the path for the window to give the compositor.
        OfferMenu { id: u64, choices: mpsc::UnboundedSender<String>, ready: futures::channel::oneshot::Sender<(String, String)> },
        Withdraw(u64),
        Show(u64, Shown),
    }

    static JOBS: Mutex<Option<std_mpsc::Sender<Job>>> = Mutex::new(None);

    fn send(job: Job) {
        let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        let tx = jobs.get_or_insert_with(|| {
            let (tx, rx) = std_mpsc::channel();
            let _ = std::thread::Builder::new().name("clay-appmenu".into()).spawn(move || serve(rx));
            tx
        });
        // The thread gave up (no session bus): nothing to offer the menu to.
        let _ = tx.send(job);
    }

    /// Offer this window's menu: on the bus, and to the desktop where to find it (X11:
    /// properties on the window; Wayland: KDE's protocol, where the compositor has it). A
    /// choice comes back to the window's `menu_action`. None where nothing would read it.
    pub(super) fn offer(window: &Window, cx: &mut Context<ClayView>) -> Option<u64> {
        let xid = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
            Ok(RawWindowHandle::Xlib(h)) => Some(h.window as u32),
            _ if super::super::wl::has_appmenu(window) => None,
            _ => return None,
        };
        let id = window.window_handle().window_id().as_u64();
        let choices = super::deliver(window, cx);
        let Some(xid) = xid else {
            let (ready, named) = futures::channel::oneshot::channel();
            send(Job::OfferMenu { id, choices, ready });
            // By the window's handle, as `deliver` goes back to it.
            let handle = window.window_handle();
            cx.spawn(async move |_, cx| {
                if let Ok((service, path)) = named.await {
                    let _ = handle.update(cx, |_, window, _| super::super::wl::set_appmenu(window, &service, &path));
                }
            })
            .detach();
            return Some(id);
        };
        send(Job::Offer { id, xid, choices });
        Some(id)
    }

    pub(super) fn show(id: u64, shown: Shown) {
        send(Job::Show(id, shown));
    }

    pub(super) fn withdraw(id: u64) {
        send(Job::Withdraw(id));
    }

    fn serve(jobs: std_mpsc::Receiver<Job>) {
        let built = zbus::block_on(async { zbus::connection::Builder::session()?.serve_at(APP_MENU_PATH, Menus)?.build().await });
        let conn = match built {
            Ok(conn) => conn,
            Err(e) => {
                crate::debug_log(true, &format!("native-gui: the desktop's app menu isn't offered (session bus: {e})"));
                return;
            }
        };
        let bus_name = conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
        let mut props = Props::default();
        for job in jobs {
            match job {
                Job::Offer { id, xid, choices } => {
                    let path = window_path(id);
                    let offered = zbus::block_on(conn.object_server().at(path.as_str(), WindowActions { choices, shown: Shown::default() }));
                    if matches!(offered, Ok(true)) {
                        props.set(xid, &bus_name, &path);
                    }
                }
                Job::OfferMenu { id, choices, ready } => {
                    let path = menu_path(id);
                    let offered = zbus::block_on(conn.object_server().at(path.as_str(), DbusMenu { choices, shown: Shown::default(), revision: 1 }));
                    if matches!(offered, Ok(true)) {
                        let _ = ready.send((bus_name.clone(), path));
                    }
                }
                // A window has one kind or the other: the one it hasn't is not there to remove.
                Job::Withdraw(id) => {
                    let _ = zbus::block_on(conn.object_server().remove::<WindowActions, _>(window_path(id).as_str()));
                    let _ = zbus::block_on(conn.object_server().remove::<DbusMenu, _>(menu_path(id).as_str()));
                }
                Job::Show(id, shown) => {
                    let path = window_path(id);
                    let _ = zbus::block_on(async {
                        let iface = conn.object_server().interface::<_, WindowActions>(path.as_str()).await?;
                        let changes = iface.get_mut().await.show(shown);
                        if !changes.is_empty() {
                            WindowActions::changed(iface.signal_emitter(), Vec::new(), changes, HashMap::new(), HashMap::new()).await?;
                        }
                        zbus::Result::Ok(())
                    });
                    let path = menu_path(id);
                    let _ = zbus::block_on(async {
                        let iface = conn.object_server().interface::<_, DbusMenu>(path.as_str()).await?;
                        let revision = iface.get_mut().await.show(shown);
                        if let Some(revision) = revision {
                            DbusMenu::layout_updated(iface.signal_emitter(), revision, ROOT).await?;
                        }
                        zbus::Result::Ok(())
                    });
                }
            }
        }
    }

    /// One `a{sv}` menu item.
    type Item = HashMap<String, OwnedValue>;

    fn text(s: &str) -> OwnedValue {
        OwnedValue::from(Str::from(s.to_string()))
    }

    fn item((action, label, shortcut): MenuEntry) -> Item {
        let mut attrs = Item::from([("label".to_string(), text(label)), ("action".to_string(), text(&format!("win.{action}")))]);
        // The hamburger's keys (F1, ...) beside the item; a command (/worlds) is no key.
        if !shortcut.is_empty() && !shortcut.starts_with('/') {
            attrs.insert("accel".to_string(), text(shortcut));
        }
        // An item a server can lack is hidden while its action is disabled.
        if action != items::QUIT.0 && !menu_always_shows(action) {
            attrs.insert("hidden-when".to_string(), text("action-disabled"));
        }
        attrs
    }

    /// `org.gtk.Menus` group 0, all there is: menu 0 is the app menu, one section per
    /// hamburger section and one for Quit; menu n is section n's items. It holds every item
    /// and never changes: the ones a server lacks come and go with their actions.
    pub(super) fn menu_group() -> Vec<(u32, u32, Vec<Item>)> {
        let sections = every();
        let root = (1..=sections.len() as u32)
            .map(|n| {
                let section = OwnedValue::try_from(Structure::from((0u32, n))).expect("two numbers make a structure");
                Item::from([(":section".to_string(), section)])
            })
            .collect();
        let mut group = vec![(0, 0, root)];
        group.extend(sections.into_iter().enumerate().map(|(n, s)| (0, n as u32 + 1, s.into_iter().map(item).collect())));
        group
    }

    struct Menus;

    #[zbus::interface(name = "org.gtk.Menus")]
    impl Menus {
        fn start(&self, groups: Vec<u32>) -> Vec<(u32, u32, Vec<Item>)> {
            if groups.contains(&0) { menu_group() } else { Vec::new() }
        }

        fn end(&self, _groups: Vec<u32>) {}
    }

    /// An action's description: enabled, parameter type (none), state (none).
    type Description = (bool, Signature, Vec<OwnedValue>);

    /// One window's actions: what its menu's items do.
    pub(super) struct WindowActions {
        choices: mpsc::UnboundedSender<String>,
        shown: Shown,
    }

    fn names() -> impl Iterator<Item = &'static str> {
        every().into_iter().flatten().map(|(action, _, _)| action)
    }

    impl WindowActions {
        fn description(&self, name: &str) -> Description {
            (self.shown.shows(name), Signature::Unit, Vec::new())
        }

        /// The server's changed: the actions it enables or disables.
        pub(super) fn show(&mut self, shown: Shown) -> HashMap<String, bool> {
            let changes = names().filter(|n| self.shown.shows(n) != shown.shows(n)).map(|n| (n.to_string(), shown.shows(n))).collect();
            self.shown = shown;
            changes
        }
    }

    #[zbus::interface(name = "org.gtk.Actions")]
    impl WindowActions {
        fn list(&self) -> Vec<String> {
            names().map(String::from).collect()
        }

        fn describe(&self, name: &str) -> zbus::fdo::Result<Description> {
            match names().any(|n| n == name) {
                true => Ok(self.description(name)),
                false => Err(zbus::fdo::Error::InvalidArgs(format!("no action {name}"))),
            }
        }

        fn describe_all(&self) -> HashMap<String, Description> {
            names().map(|n| (n.to_string(), self.description(n))).collect()
        }

        fn activate(&self, name: &str, _parameter: Vec<OwnedValue>, _platform_data: HashMap<String, OwnedValue>) {
            // A disabled item is hidden; one asked for anyway does nothing.
            if names().any(|n| n == name) && self.shown.shows(name) {
                let _ = self.choices.unbounded_send(name.to_string());
            }
        }

        fn set_state(&self, _name: &str, _value: OwnedValue, _platform_data: HashMap<String, OwnedValue>) {}

        #[zbus(signal)]
        async fn changed(
            emitter: &SignalEmitter<'_>,
            removals: Vec<String>,
            enable_changes: HashMap<String, bool>,
            state_changes: HashMap<String, OwnedValue>,
            additions: HashMap<String, Description>,
        ) -> zbus::Result<()>;
    }

    // KDE's Global Menu on Wayland: `com.canonical.dbusmenu`, one object per window.

    /// The ids: the root, the one **Clay** menu under it, an item by its place among every
    /// item there can be (so a click means the same item whatever the server shows), and
    /// the separators.
    pub(super) const ROOT: i32 = 0;
    pub(super) const CLAY: i32 = 1;
    const ITEM0: i32 = 10;
    const SEPARATOR0: i32 = 1000;

    pub(super) fn item_id(action: &str) -> Option<i32> {
        names().position(|n| n == action).map(|i| ITEM0 + i as i32)
    }

    pub(super) fn action_of(id: i32) -> Option<&'static str> {
        usize::try_from(id.checked_sub(ITEM0)?).ok().and_then(|i| names().nth(i))
    }

    /// dbusmenu marks an access key with an underscore: a literal one is doubled.
    pub(super) fn menu_label(label: &str) -> String {
        label.replace('_', "__")
    }

    /// The Clay menu's rows as `shown` has them: the hamburger's sections and Quit, a
    /// separator between two. An id and its properties.
    pub(super) fn menu_rows(shown: Shown) -> Vec<(i32, Item)> {
        let mut rows = Vec::new();
        for (n, section) in items::sections(shown).into_iter().enumerate() {
            if n > 0 {
                rows.push((SEPARATOR0 + n as i32, Item::from([("type".to_string(), text("separator"))])));
            }
            for (action, label, shortcut) in section {
                let Some(id) = item_id(action) else { continue };
                let mut props = Item::from([("label".to_string(), text(&menu_label(label)))]);
                // The hamburger's keys (F1, ...) beside the item; a command (/worlds) is no key.
                if !shortcut.is_empty() && !shortcut.starts_with('/') {
                    if let Ok(keys) = OwnedValue::try_from(Value::from(vec![vec![shortcut.to_string()]])) {
                        props.insert("shortcut".to_string(), keys);
                    }
                }
                rows.push((id, props));
            }
        }
        rows
    }

    /// The root and the Clay menu, then its rows.
    fn menu_items(shown: Shown) -> Vec<(i32, Item)> {
        let submenu = ("children-display".to_string(), text("submenu"));
        let mut all = vec![(ROOT, Item::from([submenu.clone()])), (CLAY, Item::from([("label".to_string(), text("Clay")), submenu]))];
        all.extend(menu_rows(shown));
        all
    }

    /// The properties asked for: all of them when none is named.
    fn only(mut props: Item, names: &[String]) -> Item {
        if !names.is_empty() {
            props.retain(|k, _| names.iter().any(|n| n == k));
        }
        props
    }

    /// A menu item with what is under it: `(ia{sv}av)`, each child a variant of the same.
    pub(super) type Node = (i32, Item, Vec<OwnedValue>);

    fn child((id, props, children): Node) -> Option<OwnedValue> {
        OwnedValue::try_from(Value::from(Structure::from((id, props, children)))).ok()
    }

    /// `parent` with its children `depth` levels down (negative: all the way).
    pub(super) fn menu_layout(shown: Shown, parent: i32, depth: i32, names: &[String]) -> Option<Node> {
        let mut items = menu_items(shown).into_iter().map(|(id, props)| (id, only(props, names)));
        let rows = |depth: i32, items: &mut dyn Iterator<Item = (i32, Item)>| -> Vec<OwnedValue> {
            if depth == 0 { Vec::new() } else { items.filter_map(|(id, props)| child((id, props, Vec::new()))).collect() }
        };
        match parent {
            ROOT => {
                let (_, root) = items.next()?;
                let (_, clay) = items.next()?;
                let under = if depth == 0 { Vec::new() } else { child((CLAY, clay, rows(depth - 1, &mut items))).into_iter().collect() };
                Some((ROOT, root, under))
            }
            CLAY => {
                let (_, clay) = items.nth(1)?;
                Some((CLAY, clay, rows(depth, &mut items)))
            }
            id => items.find(|(i, _)| *i == id).map(|(i, props)| (i, props, Vec::new())),
        }
    }

    /// One window's menu, as KDE reads it on Wayland.
    pub(super) struct DbusMenu {
        pub(super) choices: mpsc::UnboundedSender<String>,
        pub(super) shown: Shown,
        /// Goes up when the layout changes; a reader keeps the one it last fetched.
        pub(super) revision: u32,
    }

    impl DbusMenu {
        /// The server's changed: the new revision, when the menu is another one.
        fn show(&mut self, shown: Shown) -> Option<u32> {
            if self.shown == shown {
                return None;
            }
            self.shown = shown;
            self.revision += 1;
            Some(self.revision)
        }

        /// A click on an item this server's menu has; anything else does nothing.
        pub(super) fn clicked(&self, id: i32, event: &str) {
            if event != "clicked" {
                return;
            }
            if let Some(action) = action_of(id).filter(|a| self.shown.shows(a)) {
                let _ = self.choices.unbounded_send(action.to_string());
            }
        }
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl DbusMenu {
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }

        #[zbus(property)]
        fn text_direction(&self) -> String {
            "ltr".to_string()
        }

        #[zbus(property)]
        fn status(&self) -> String {
            "normal".to_string()
        }

        #[zbus(property)]
        fn icon_theme_path(&self) -> Vec<String> {
            Vec::new()
        }

        fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<String>) -> zbus::fdo::Result<(u32, Node)> {
            match menu_layout(self.shown, parent_id, recursion_depth, &property_names) {
                Some(node) => Ok((self.revision, node)),
                None => Err(zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}"))),
            }
        }

        fn get_group_properties(&self, ids: Vec<i32>, property_names: Vec<String>) -> Vec<(i32, Item)> {
            menu_items(self.shown).into_iter().filter(|(id, _)| ids.is_empty() || ids.contains(id)).map(|(id, props)| (id, only(props, &property_names))).collect()
        }

        fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
            let found = menu_items(self.shown).into_iter().find(|(i, _)| *i == id).and_then(|(_, mut props)| props.remove(name));
            found.ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on menu item {id}")))
        }

        fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
            self.clicked(id, event_id);
        }

        /// The ids that weren't found: none is reported, a stale id's click is dropped.
        fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
            for (id, event_id, _, _) in &events {
                self.clicked(*id, event_id);
            }
            Vec::new()
        }

        /// Nothing is built on demand: the layout a reader has is the one there is.
        fn about_to_show(&self, _id: i32) -> bool {
            false
        }

        fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
            (Vec::new(), Vec::new())
        }

        #[zbus(signal)]
        async fn items_properties_updated(emitter: &SignalEmitter<'_>, updated: Vec<(i32, Item)>, removed: Vec<(i32, Vec<String>)>) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn item_activation_requested(emitter: &SignalEmitter<'_>, id: i32, timestamp: u32) -> zbus::Result<()>;
    }

    /// The window properties that say where its menu is, on one X connection.
    #[derive(Default)]
    struct Props(Option<(RustConnection, [Atom; 3], Atom, [Atom; 2])>);

    impl Props {
        fn set(&mut self, xid: u32, bus_name: &str, window_path: &str) {
            if self.0.is_none() {
                self.0 = connect();
            }
            let Some((conn, [bus, menu, win], utf8, [kind, normal])) = &self.0 else { return };
            let sent = [(*bus, bus_name), (*menu, APP_MENU_PATH), (*win, window_path)]
                .into_iter()
                .try_for_each(|(atom, value)| conn.change_property8(PropMode::REPLACE, xid, atom, *utf8, value.as_bytes()).map(|_| ()))
                // An ordinary window, said out loud: GPUI leaves the type unset, and KDE
                // Plasma's menu proxy (gmenu-dbusmenu-proxy) passes over a window without
                // one, so Plasma's Global Menu never showed Clay's.
                .and_then(|_| conn.change_property32(PropMode::REPLACE, xid, *kind, AtomEnum::ATOM, &[*normal]).map(|_| ()));
            if sent.and_then(|_| conn.flush()).is_err() {
                // The display went away: start again next time.
                self.0 = None;
            }
        }
    }

    fn connect() -> Option<(RustConnection, [Atom; 3], Atom, [Atom; 2])> {
        let (conn, _) = RustConnection::connect(None).ok()?;
        let atom = |name: &[u8]| conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom);
        let atoms = [atom(b"_GTK_UNIQUE_BUS_NAME")?, atom(b"_GTK_APP_MENU_OBJECT_PATH")?, atom(b"_GTK_WINDOW_OBJECT_PATH")?];
        let utf8 = atom(b"UTF8_STRING")?;
        let kind = [atom(b"_NET_WM_WINDOW_TYPE")?, atom(b"_NET_WM_WINDOW_TYPE_NORMAL")?];
        Some((conn, atoms, utf8, kind))
    }

    #[cfg(test)]
    mod tests {
        use super::{Shown, WindowActions, menu_group};
        use zbus::zvariant::{OwnedValue, Str, Structure};

        fn text(v: &OwnedValue) -> String {
            Str::try_from(v.clone()).map(|s| s.to_string()).unwrap_or_default()
        }

        /// The model: a section per hamburger section and one for Quit, each item's action
        /// the window's, F-keys as accelerators, and the items a server can lack hidden
        /// while disabled.
        #[test]
        fn the_menu_is_the_hamburgers_then_quit() {
            let group = menu_group();
            let (g, m, root) = &group[0];
            assert_eq!((*g, *m), (0, 0));
            assert_eq!(root.len(), 5, "four hamburger sections and Quit");
            let first = Structure::try_from(root[0][":section"].clone()).expect("a section link");
            assert_eq!(first.fields().len(), 2);
            let labels: Vec<Vec<String>> = group[1..].iter().map(|(_, _, items)| items.iter().map(|i| text(&i["label"])).collect()).collect();
            assert_eq!(labels[0], ["Help", "World Selector", "Edit World", "Actions", "Settings", "Map", "Emoji Picker", "Import Picture"]);
            assert_eq!(labels[4], ["Quit"]);
            let help = &group[1].2[0];
            assert_eq!(text(&help["action"]), "win.help");
            assert_eq!(text(&help["accel"]), "F1");
            assert!(!group[1].2[1].contains_key("accel"), "a command isn't a key");
            let hidden: Vec<String> = group[1..].iter().flat_map(|(_, _, items)| items.iter().filter(|i| i.contains_key("hidden-when")).map(|i| text(&i["action"]))).collect();
            // Edit World and Map are always shown since the clay_multi redesign (2026-11-09:
            // a user's worlds are their own); Import Picture, Change Password and Logout
            // still depend on the server.
            assert_eq!(hidden, ["win.import-picture", "win.change-password", "win.logout"]);
        }

        /// A server that lacks something disables its actions, and a change says which.
        #[test]
        fn actions_follow_the_server() {
            let (choices, _chosen) = futures::channel::mpsc::unbounded();
            let mut actions = WindowActions { choices, shown: Shown::default() };
            assert!(!actions.description("logout").0);
            assert!(actions.description("quit").0);
            assert!(actions.description("edit-world").0);
            let changes = actions.show(Shown { multiuser: true, picture_upload: false, change_password: true });
            assert_eq!(changes.len(), 2, "{changes:?}");
            assert_eq!(changes.get("logout"), Some(&true));
            assert_eq!(changes.get("change-password"), Some(&true));
            assert!(actions.show(Shown { multiuser: true, picture_upload: false, change_password: true }).is_empty(), "no change, nothing to say");
            let changes = actions.show(Shown { multiuser: true, picture_upload: false, change_password: false });
            assert_eq!(changes.get("change-password"), Some(&false), "clay_multi: password.dat is the operator's");
        }

        use super::{CLAY, DbusMenu, ROOT, action_of, item_id, menu_label, menu_layout, menu_rows};

        /// A node's id, and its children's nodes.
        fn node_id(v: &OwnedValue) -> i32 {
            let s = Structure::try_from(v.clone()).expect("a child is a structure");
            i32::try_from(&s.fields()[0]).expect("which starts with its id")
        }

        /// KDE's menu is the hamburger's list for every kind of server: its sections in
        /// order with a separator between two, then Quit, each item under an id that means
        /// that item whatever the server shows.
        #[test]
        fn the_kde_menu_is_the_hamburgers_for_every_server() {
            for shown in Shown::every() {
                let rows = menu_rows(shown);
                let mut want = Vec::new();
                for (n, section) in super::items::sections(shown).into_iter().enumerate() {
                    if n > 0 {
                        want.push("-".to_string());
                    }
                    want.extend(section.iter().map(|(_, label, _)| label.to_string()));
                }
                let got: Vec<String> = rows.iter().map(|(_, p)| if p.get("type").map(text).as_deref() == Some("separator") { "-".to_string() } else { p.get("label").map(text).unwrap_or_default() }).collect();
                assert_eq!(got, want, "{shown:?}");
                assert_eq!(want.last().map(String::as_str), Some("Quit"));
                let ids: std::collections::HashSet<i32> = rows.iter().map(|(id, _)| *id).collect();
                assert_eq!(ids.len(), rows.len(), "distinct ids: {shown:?}");
                assert!(!ids.contains(&ROOT) && !ids.contains(&CLAY));
                for (action, _, _) in super::items::sections(shown).into_iter().flatten() {
                    let id = item_id(action).expect("every item has an id");
                    assert_eq!(action_of(id), Some(action));
                    assert!(ids.contains(&id), "{action} is in the menu under its id");
                }
            }
            let single = item_id("help");
            assert!(Shown::every().into_iter().all(|_| item_id("help") == single), "an id never depends on the server");
            assert_eq!(action_of(ROOT), None);
            assert_eq!(action_of(CLAY), None);
            assert_eq!(action_of(1001), None, "a separator is no action");
            let help = menu_rows(Shown::default()).into_iter().find(|(id, _)| Some(*id) == single).expect("Help is there").1;
            assert!(help.contains_key("shortcut"), "F1 beside Help");
            assert_eq!(menu_label("a_b"), "a__b", "an underscore would be an access key");
        }

        /// What a reader asks for: the root holds the one Clay menu, which holds the rows;
        /// a depth stops there, and named properties are the only ones sent.
        #[test]
        fn the_kde_menu_layout_is_one_clay_menu_and_honours_depth() {
            let shown = Shown::default();
            let rows: Vec<i32> = menu_rows(shown).into_iter().map(|(id, _)| id).collect();
            let (id, props, children) = menu_layout(shown, ROOT, -1, &[]).expect("the root");
            assert_eq!(id, ROOT);
            assert_eq!(props.get("children-display").map(text).as_deref(), Some("submenu"));
            assert_eq!(children.iter().map(node_id).collect::<Vec<_>>(), vec![CLAY]);
            let clay = Structure::try_from(children[0].clone()).unwrap();
            let under = zbus::zvariant::Array::try_from(clay.fields()[2].clone()).expect("children");
            assert_eq!(under.len(), rows.len(), "everything under Clay");

            assert!(menu_layout(shown, ROOT, 0, &[]).unwrap().2.is_empty(), "depth 0: the item alone");
            let (_, _, one) = menu_layout(shown, ROOT, 1, &[]).unwrap();
            let clay = Structure::try_from(one[0].clone()).unwrap();
            assert_eq!(zbus::zvariant::Array::try_from(clay.fields()[2].clone()).unwrap().len(), 0, "depth 1: Clay without its rows");

            let (id, props, children) = menu_layout(shown, CLAY, -1, &["label".to_string()]).expect("the Clay menu");
            assert_eq!((id, props.get("label").map(text).as_deref()), (CLAY, Some("Clay")));
            assert!(!props.contains_key("children-display"), "only the property asked for");
            assert_eq!(children.iter().map(node_id).collect::<Vec<_>>(), rows);
            let help = item_id("help").unwrap();
            assert_eq!(menu_layout(shown, help, -1, &[]).map(|(id, _, c)| (id, c.len())), Some((help, 0)));
            assert!(menu_layout(shown, 9999, -1, &[]).is_none());
        }

        /// A click runs the item in its window; an item this server's menu lacks, another
        /// event or an unknown id does nothing. A new server means a new revision.
        #[test]
        fn a_kde_menu_click_runs_only_what_the_menu_shows() {
            let (choices, mut chosen) = futures::channel::mpsc::unbounded();
            let mut menu = DbusMenu { choices, shown: Shown::default(), revision: 1 };
            menu.clicked(item_id("help").unwrap(), "clicked");
            menu.clicked(item_id("help").unwrap(), "hovered");
            menu.clicked(item_id("logout").unwrap(), "clicked");
            menu.clicked(CLAY, "clicked");
            menu.clicked(1001, "clicked");
            menu.clicked(item_id("quit").unwrap(), "clicked");
            let mut got = Vec::new();
            while let Ok(a) = chosen.try_recv() {
                got.push(a);
            }
            assert_eq!(got, vec!["help".to_string(), "quit".to_string()]);
            assert_eq!(menu.show(Shown::default()), None, "the same server: the same menu");
            assert_eq!(menu.show(Shown { multiuser: true, picture_upload: false, change_password: true }), Some(2));
            menu.clicked(item_id("logout").unwrap(), "clicked");
            assert_eq!(chosen.try_recv().ok().as_deref(), Some("logout"));
        }
    }
}
