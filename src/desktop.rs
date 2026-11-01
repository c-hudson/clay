//! What the two desktop GUIs show outside their windows (user request 2026-10-31): a taskbar
//! alert when text arrives while no Clay window has focus (Settings > Taskbar Alert), the
//! activity count on the taskbar button or Dock icon, and `/notify` as a desktop
//! notification. One implementation for the native GUI and the WebView GUI; each GUI decides
//! *when* (the native GUI's window with the connection, the WebView GUI's main window) and
//! calls in here. The console, the browser page and the Android app have none of this (user
//! ruling: they can't reach a taskbar, and Android already notifies).
//!
//! - Linux: one session-bus connection on a thread of its own. The badge is Unity's
//!   `com.canonical.Unity.LauncherEntry` (KDE's task manager, Ubuntu Dock / Dash to Dock,
//!   Plank; xfce's tasklist shows none), a notification `org.freedesktop.Notifications`. A
//!   dock attaches the badge to the `.desktop` file it names, matched to the window by its
//!   class, so both GUIs name their windows `clay` and `ensure_desktop_file` writes
//!   `clay.desktop`.
//! - macOS: the Dock tile's badge label; a notification through `osascript`, the one way a
//!   binary outside an .app bundle has (it shows as Script Editor's).
//! - Windows: the taskbar button's overlay icon (`ITaskbarList3`), and a toast from the
//!   AppUserModelID `Clay`, registered once under HKCU as GPUI does: no shortcut, no tray icon.

use std::sync::atomic::{AtomicBool, Ordering};

/// What a badge shows for `count` worlds with activity; nothing for none.
pub fn badge_text(count: usize) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".to_string()),
    }
}

static ALERTED: AtomicBool = AtomicBool::new(false);

/// Whether to ask for attention now: once in each stretch without a Clay window focused,
/// whichever window asks.
pub fn alert_once() -> bool {
    !ALERTED.swap(true, Ordering::SeqCst)
}

/// A Clay window has focus: the next stretch without one may alert again.
pub fn alert_reset() {
    ALERTED.store(false, Ordering::SeqCst);
}

/// Show `count` worlds with activity on the taskbar or the Dock. `hwnd` is the window whose
/// taskbar button carries it on Windows; macOS's badge must be set on the main thread, as
/// both GUIs call this.
pub fn set_badge(count: usize, hwnd: Option<isize>) {
    let _ = hwnd;
    #[cfg(target_os = "linux")]
    linux::send(linux::Job::Badge(count));
    #[cfg(windows)]
    if let Some(hwnd) = hwnd {
        win::send(win::Job::Overlay { hwnd, count });
    }
    #[cfg(target_os = "macos")]
    mac::set_badge(count);
}

/// `/notify`'s message as a desktop notification, titled with its world's name.
pub fn notify(title: &str, body: &str) {
    let _ = (title, body);
    #[cfg(target_os = "linux")]
    linux::send(linux::Job::Notify { title: title.to_string(), body: body.to_string() });
    #[cfg(windows)]
    win::send(win::Job::Notify { title: title.to_string(), body: body.to_string() });
    #[cfg(target_os = "macos")]
    mac::notify(title, body);
}

/// Linux: write `clay.desktop` (and Clay's icon) for the docks, on a thread of its own.
pub fn ensure_desktop_file() {
    #[cfg(target_os = "linux")]
    desktop_file::ensure();
}

/// The digits and "+", 5x7, a row per byte (bit 4 is the leftmost column).
#[cfg_attr(not(windows), allow(dead_code))]
const GLYPHS: [[u8; 7]; 11] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
    [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
];

/// Windows' overlay icon for `count`: white digits on a red disc, 32x32 RGBA (Windows draws
/// it at the small-icon size). A one-cell overlay has no room for more than "9+".
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn badge_icon_rgba(count: usize) -> (Vec<u8>, u32, u32) {
    const SIZE: usize = 32;
    let mut px = vec![0u8; SIZE * SIZE * 4];
    if count == 0 {
        return (px, SIZE as u32, SIZE as u32);
    }
    let mut put = |x: usize, y: usize, rgba: [u8; 4]| px[(y * SIZE + x) * 4..][..4].copy_from_slice(&rgba);
    // The disc, its edge smoothed with 4x4 samples a pixel.
    let r = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let inside = (0..16)
                .filter(|s| {
                    let dx = x as f32 + ((s % 4) as f32 + 0.5) / 4.0 - r;
                    let dy = y as f32 + ((s / 4) as f32 + 0.5) / 4.0 - r;
                    dx * dx + dy * dy <= r * r
                })
                .count();
            if inside > 0 {
                put(x, y, [0xD3, 0x2F, 0x2F, (inside * 255 / 16) as u8]);
            }
        }
    }
    let glyphs: Vec<usize> = if count > 9 { vec![9, 10] } else { vec![count] };
    let scale = if glyphs.len() == 1 { 3 } else { 2 };
    let width = glyphs.len() * 5 * scale + (glyphs.len() - 1) * scale;
    let (x0, y0) = ((SIZE - width) / 2, (SIZE - 7 * scale) / 2);
    for (i, &g) in glyphs.iter().enumerate() {
        let left = x0 + i * 6 * scale;
        for (row, bits) in GLYPHS[g].iter().enumerate() {
            for col in 0..5 {
                if bits >> (4 - col) & 1 == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            put(left + col * scale + dx, y0 + row * scale + dy, [0xFF, 0xFF, 0xFF, 0xFF]);
                        }
                    }
                }
            }
        }
    }
    (px, SIZE as u32, SIZE as u32)
}

/// A Windows toast: the world's name, then the message. XML 1.0 allows no C0 controls but
/// tab and the line ends, and `LoadXml` refuses a document with one.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn toast_xml(title: &str, body: &str) -> String {
    fn escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
                c => out.push(c),
            }
        }
        out
    }
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        escape(title),
        escape(body)
    )
}

/// `osascript`'s arguments for a notification. The text goes in as the script's own
/// arguments, so nothing in it needs quoting.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn osascript_args(title: &str, body: &str) -> Vec<String> {
    ["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", title, body]
        .into_iter()
        .map(String::from)
        .collect()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{OwnedValue, Value};

    /// The `.desktop` file a dock attaches the badge to (`desktop_file`).
    pub(super) const APP_URI: &str = "application://clay.desktop";
    const LAUNCHER_PATH: &str = "/org/clay/Clay/launcher";

    pub(super) enum Job {
        Badge(usize),
        Notify { title: String, body: String },
    }

    static JOBS: Mutex<Option<mpsc::Sender<Job>>> = Mutex::new(None);

    pub(super) fn send(job: Job) {
        let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        let tx = jobs.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let _ = std::thread::Builder::new().name("clay-desktop".into()).spawn(move || serve(rx));
            tx
        });
        // The thread gave up (no session bus): nobody to show anything to.
        let _ = tx.send(job);
    }

    /// The badge's properties: the count, shown while there is one.
    pub(super) fn properties(count: usize) -> HashMap<String, OwnedValue> {
        HashMap::from([("count".to_string(), OwnedValue::from(count as i64)), ("count-visible".to_string(), OwnedValue::from(count > 0))])
    }

    /// The badge, also answering a dock that starts after it was set (`Query`).
    struct Launcher {
        count: usize,
    }

    #[zbus::interface(name = "com.canonical.Unity.LauncherEntry")]
    impl Launcher {
        fn query(&self) -> (String, HashMap<String, OwnedValue>) {
            (APP_URI.to_string(), properties(self.count))
        }

        #[zbus(signal)]
        async fn update(emitter: &SignalEmitter<'_>, app_uri: &str, properties: HashMap<String, OwnedValue>) -> zbus::Result<()>;
    }

    /// `<`, `>` and `&` would be markup in a notification's body.
    pub(super) fn markup_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn serve(jobs: mpsc::Receiver<Job>) {
        let built = zbus::block_on(async { zbus::connection::Builder::session()?.serve_at(LAUNCHER_PATH, Launcher { count: 0 })?.build().await });
        let conn = match built {
            Ok(conn) => conn,
            Err(e) => {
                crate::debug_log(true, &format!("desktop: no badge or notifications (session bus: {e})"));
                return;
            }
        };
        for job in jobs {
            match job {
                Job::Badge(count) => {
                    let _ = zbus::block_on(async {
                        let iface = conn.object_server().interface::<_, Launcher>(LAUNCHER_PATH).await?;
                        iface.get_mut().await.count = count;
                        Launcher::update(iface.signal_emitter(), APP_URI, properties(count)).await
                    });
                }
                Job::Notify { title, body } => {
                    let hints = HashMap::from([("desktop-entry", Value::from("clay"))]);
                    let body = markup_escape(&body);
                    let args = ("Clay", 0u32, "clay", title.as_str(), body.as_str(), Vec::<&str>::new(), hints, -1i32);
                    let sent = zbus::block_on(conn.call_method(
                        Some("org.freedesktop.Notifications"),
                        "/org/freedesktop/Notifications",
                        Some("org.freedesktop.Notifications"),
                        "Notify",
                        &args,
                    ));
                    if let Err(e) = sent {
                        crate::debug_log(true, &format!("desktop: notification not shown: {e}"));
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod desktop_file {
    use std::path::{Path, PathBuf};

    /// Only a file carrying this is Clay's to rewrite.
    pub(super) const MARK: &str = "X-Clay-Generated=true";
    const ICON: &[u8] = include_bytes!("../clay_icon.png");

    pub(super) fn ensure() {
        let _ = std::thread::Builder::new().name("clay-desktop-file".into()).spawn(|| {
            let Ok(exe) = std::env::current_exe() else { return };
            match write(&data_home(), &exe) {
                Ok(Some(done)) => crate::debug_log(true, &format!("desktop: {done}")),
                Ok(None) => {}
                Err(e) => crate::debug_log(true, &format!("desktop: clay.desktop not written: {e}")),
            }
        });
    }

    fn data_home() -> PathBuf {
        match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            Some(dir) if dir.is_absolute() => dir,
            _ => PathBuf::from(crate::get_home_dir()).join(".local/share"),
        }
    }

    /// A binary in a cargo `target` directory: a developer's build, which must not take over
    /// the launcher entry of the Clay actually installed.
    pub(super) fn is_build_output(exe: &Path) -> bool {
        exe.ancestors().skip(1).take(3).any(|dir| dir.file_name().is_some_and(|n| n == "target"))
    }

    /// The program as an `Exec` argument: quoted, `"` `` ` `` `$` and `\` escaped for the
    /// quoting and that backslash escaped again for the string, `%` doubled (a field code).
    pub(super) fn exec_quote(exe: &Path) -> String {
        let mut quoted = String::from("\"");
        for c in exe.to_string_lossy().chars() {
            match c {
                '\\' => quoted.push_str(r"\\\\"),
                '"' | '`' | '$' => {
                    quoted.push_str(r"\\");
                    quoted.push(c);
                }
                '%' => quoted.push_str("%%"),
                c => quoted.push(c),
            }
        }
        quoted.push('"');
        quoted
    }

    pub(super) fn text(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Clay\nGenericName=MUD Client\nComment=A MUD client\nExec={} --gui\nIcon=clay\nTerminal=false\nCategories=Network;Game;\nStartupWMClass=clay\n{MARK}\n",
            exec_quote(exe)
        )
    }

    /// Write `applications/clay.desktop` under `data` when it is missing, or when it is Clay's
    /// and says something else (another binary); a file without `MARK` is someone else's and
    /// stays as it is. Clay's icon goes beside it in the hicolor theme. Says what it did.
    pub(super) fn write(data: &Path, exe: &Path) -> std::io::Result<Option<&'static str>> {
        let shown = exe.to_string_lossy();
        // A binary replaced while it runs reads back as "<path> (deleted)".
        let exe = Path::new(shown.strip_suffix(" (deleted)").unwrap_or(&shown));
        if is_build_output(exe) {
            return Ok(None);
        }
        let file = data.join("applications").join("clay.desktop");
        let want = text(exe);
        let done = match std::fs::read_to_string(&file) {
            Ok(have) if !have.lines().any(|l| l.trim() == MARK) => return Ok(None),
            Ok(have) if have == want => None,
            Ok(_) => Some("clay.desktop updated"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some("clay.desktop written"),
            Err(e) => return Err(e),
        };
        if done.is_some() {
            replace(&file, want.as_bytes())?;
        }
        let icon = data.join("icons/hicolor/256x256/apps/clay.png");
        if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
            replace(&icon, ICON)?;
        }
        Ok(done)
    }

    /// Through a temporary file, so a launcher never reads half of one.
    fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("clay-tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(target_os = "macos")]
mod mac {
    #[allow(deprecated)]
    use objc2_foundation::MainThreadMarker;
    use objc2_foundation::NSString;

    pub(super) fn set_badge(count: usize) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let label = super::badge_text(count).map(|s| NSString::from_str(&s));
        objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
    }

    pub(super) fn notify(title: &str, body: &str) {
        let args = super::osascript_args(title, body);
        let _ = std::thread::Builder::new().name("clay-notify".into()).spawn(move || {
            let ran = std::process::Command::new("osascript")
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if let Err(e) = ran {
                crate::debug_log(true, &format!("desktop: notification not shown (osascript: {e})"));
            }
        });
    }
}

#[cfg(windows)]
mod win {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};
    use windows::core::{HSTRING, PCWSTR};

    /// The toasts' AppUserModelID, registered under HKCU (`register`).
    const APP_ID: &str = "Clay";
    const ICON: &[u8] = include_bytes!("../clay_icon.png");

    pub(super) enum Job {
        Overlay { hwnd: isize, count: usize },
        Notify { title: String, body: String },
    }

    static JOBS: Mutex<Option<mpsc::Sender<Job>>> = Mutex::new(None);

    pub(super) fn send(job: Job) {
        let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        let tx = jobs.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let _ = std::thread::Builder::new().name("clay-desktop".into()).spawn(move || serve(rx));
            tx
        });
        let _ = tx.send(job);
    }

    fn serve(jobs: mpsc::Receiver<Job>) {
        // SAFETY: once, on this thread, before any COM call on it.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let mut taskbar: Option<ITaskbarList3> = None;
        let mut notifier: Option<ToastNotifier> = None;
        for job in jobs {
            match job {
                Job::Overlay { hwnd, count } => {
                    if taskbar.is_none() {
                        // SAFETY: COM is initialised on this thread.
                        let made = unsafe { CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) }.and_then(|t| unsafe { t.HrInit() }.map(|_| t));
                        match made {
                            Ok(t) => taskbar = Some(t),
                            Err(e) => {
                                crate::debug_log(true, &format!("desktop: no taskbar badge ({e})"));
                                continue;
                            }
                        }
                    }
                    if let Some(Err(e)) = taskbar.as_ref().map(|t| overlay(t, HWND(hwnd as *mut core::ffi::c_void), count)) {
                        crate::debug_log(true, &format!("desktop: taskbar badge not set ({e})"));
                    }
                }
                Job::Notify { title, body } => {
                    if notifier.is_none() {
                        register();
                        match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)) {
                            Ok(n) => notifier = Some(n),
                            Err(e) => {
                                crate::debug_log(true, &format!("desktop: notification not shown ({e})"));
                                continue;
                            }
                        }
                    }
                    if let Some(Err(e)) = notifier.as_ref().map(|n| show(n, &title, &body)) {
                        crate::debug_log(true, &format!("desktop: notification not shown ({e})"));
                    }
                }
            }
        }
    }

    fn overlay(taskbar: &ITaskbarList3, hwnd: HWND, count: usize) -> windows::core::Result<()> {
        let Some(label) = super::badge_text(count) else {
            // SAFETY: a null icon clears the overlay.
            return unsafe { taskbar.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null()) };
        };
        let (rgba, w, h) = super::badge_icon_rgba(count);
        let bgra: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        // All zeros: the colour bitmap's alpha decides what shows.
        let mask = vec![0u8; (w * h / 8) as usize];
        // SAFETY: both buffers are the size the dimensions say.
        let icon = unsafe { CreateIcon(None, w as i32, h as i32, 1, 32, mask.as_ptr(), bgra.as_ptr()) }?;
        let description = HSTRING::from(format!("{label} with activity"));
        // SAFETY: the taskbar keeps its own copy of the icon.
        let set = unsafe { taskbar.SetOverlayIcon(hwnd, icon, &description) };
        let _ = unsafe { DestroyIcon(icon) };
        set
    }

    /// The AppUserModelID's name and icon, so the toasts say Clay: an unpackaged program
    /// without a Start-menu shortcut has nothing else to show them under.
    fn register() {
        let key = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
        let set = |name: &str, value: &str| {
            let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string of the length given.
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(name), REG_SZ.0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32) }
        };
        let _ = set("DisplayName", "Clay");
        let icon = crate::clay_config_dir().join("clay_icon.png");
        if std::fs::read(&icon).ok().as_deref() == Some(ICON) || std::fs::write(&icon, ICON).is_ok() {
            let _ = set("IconUri", &icon.to_string_lossy());
        }
    }

    fn show(notifier: &ToastNotifier, title: &str, body: &str) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(super::toast_xml(title, body)))?;
        notifier.Show(&ToastNotification::CreateToastNotification(&doc)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_badge_shows_the_count_and_nothing_for_none() {
        assert_eq!(badge_text(0), None);
        assert_eq!(badge_text(3).as_deref(), Some("3"));
        assert_eq!(badge_text(99).as_deref(), Some("99"));
        assert_eq!(badge_text(100).as_deref(), Some("99+"));
    }

    #[test]
    fn alerts_once_until_focus_returns() {
        alert_reset();
        assert!(alert_once());
        assert!(!alert_once(), "a second arrival in the same stretch doesn't alert");
        alert_reset();
        assert!(alert_once());
        alert_reset();
    }

    /// 32x32 RGBA: nothing at all for no count; a red disc with white in its middle, and
    /// clear corners, for one.
    #[test]
    fn the_overlay_icon_is_digits_on_a_disc() {
        let (px, w, h) = badge_icon_rgba(0);
        assert_eq!((w, h, px.len()), (32, 32, 32 * 32 * 4));
        assert!(px.iter().all(|&b| b == 0));
        let at = |px: &[u8], x: usize, y: usize| px[(y * 32 + x) * 4..][..4].to_vec();
        for count in [1, 7, 12, 500] {
            let (px, _, _) = badge_icon_rgba(count);
            assert_eq!(at(&px, 0, 0)[3], 0, "corners are clear");
            assert_eq!(at(&px, 16, 2), [0xD3, 0x2F, 0x2F, 0xFF], "the disc is red");
            let white = (0..32 * 32).filter(|i| px[i * 4..][..4] == [0xFF; 4]).count();
            assert!(white > 20, "{count}: the digits are drawn ({white} white pixels)");
        }
        assert_eq!(badge_icon_rgba(12).0, badge_icon_rgba(500).0, "both are 9+");
    }

    #[test]
    fn a_toast_escapes_its_text() {
        let xml = toast_xml("Mud & <Co>", "said \"hi\"\x1b[0m it's");
        assert_eq!(
            xml,
            r#"<toast><visual><binding template="ToastGeneric"><text>Mud &amp; &lt;Co&gt;</text><text>said &quot;hi&quot;[0m it&apos;s</text></binding></visual></toast>"#
        );
    }

    #[test]
    fn osascript_takes_the_text_as_arguments() {
        let args = osascript_args("World \"x\"", "a 'b'");
        assert_eq!(args.len(), 8);
        assert_eq!(args[6], "World \"x\"");
        assert_eq!(args[7], "a 'b'");
        assert!(args[3].contains("item 2 of argv"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_launcher_entry_counts_and_shows_while_counting() {
        use zbus::zvariant::OwnedValue;
        let p = linux::properties(4);
        assert_eq!(p.get("count"), Some(&OwnedValue::from(4i64)));
        assert_eq!(p.get("count-visible"), Some(&OwnedValue::from(true)));
        assert_eq!(linux::properties(0).get("count-visible"), Some(&OwnedValue::from(false)));
        assert_eq!(linux::APP_URI, "application://clay.desktop");
        assert_eq!(linux::markup_escape("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_desktop_file_is_written_once_and_never_over_someone_elses() {
        use std::path::Path;
        let dir = std::env::temp_dir().join(format!("clay-desktop-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("applications/clay.desktop");
        let exe = Path::new("/opt/My Clay/clay");
        assert_eq!(desktop_file::write(&dir, exe).unwrap(), Some("clay.desktop written"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Exec=\"/opt/My Clay/clay\" --gui\n"), "{text}");
        assert!(text.contains("StartupWMClass=clay\n") && text.contains("Icon=clay\n"));
        assert!(dir.join("icons/hicolor/256x256/apps/clay.png").exists());
        assert_eq!(desktop_file::write(&dir, exe).unwrap(), None, "unchanged");
        assert_eq!(desktop_file::write(&dir, Path::new("/usr/bin/clay")).unwrap(), Some("clay.desktop updated"));
        assert!(std::fs::read_to_string(&file).unwrap().contains("Exec=\"/usr/bin/clay\" --gui"));
        assert_eq!(desktop_file::write(&dir, Path::new("/usr/bin/clay (deleted)")).unwrap(), None, "a replaced binary is the same one");
        std::fs::write(&file, "[Desktop Entry]\nName=My Clay\nExec=myclay\n").unwrap();
        assert_eq!(desktop_file::write(&dir, exe).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[Desktop Entry]\nName=My Clay\nExec=myclay\n", "someone else's file stays");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(desktop_file::write(&dir, Path::new("/home/u/src/clay/target/release/clay")).unwrap(), None, "a build doesn't take the entry");
        assert_eq!(desktop_file::write(&dir, Path::new("/home/u/src/clay/target/x86_64-unknown-linux-musl/debug/clay")).unwrap(), None);
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exec_quotes_what_the_spec_reserves() {
        use std::path::Path;
        assert_eq!(desktop_file::exec_quote(Path::new("/a b/clay")), "\"/a b/clay\"");
        assert_eq!(desktop_file::exec_quote(Path::new("/a$b/c\"d/50%")), r#""/a\\$b/c\\"d/50%%""#);
        assert_eq!(desktop_file::exec_quote(Path::new(r"/a\b")), r#""/a\\\\b""#);
    }
}
