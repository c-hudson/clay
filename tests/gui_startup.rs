//! Starts the real desktop GUI (`clay --gui`) on a headless GTK Broadway display and
//! checks what it prints to the terminal.
//!
//! Only built with the GUI (`cargo test --features webview-gui`) on Linux, and skipped
//! (with a note) when `broadwayd` isn't installed.

#![cfg(all(feature = "webview-gui", target_os = "linux"))]

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Kills the child when the test ends, however it ends.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A broadwayd on the first display number that is free.
fn start_broadway() -> Option<(KillOnDrop, u32)> {
    for n in 40..90u32 {
        // broadwayd serves display :n on port 8080+n; skip numbers someone is using.
        if std::net::TcpListener::bind(("127.0.0.1", (8080 + n) as u16)).is_err() {
            continue;
        }
        let child = Command::new("broadwayd")
            .arg(format!(":{n}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut guard = KillOnDrop(child);
        std::thread::sleep(Duration::from_millis(700));
        if guard.0.try_wait().ok().flatten().is_none() {
            return Some((guard, n));
        }
    }
    None
}

/// WebKit's JavaScriptCore wants SIGUSR1 for garbage collection, and Clay owns SIGUSR1
/// (hot reload), so when WebKit starts up in the GUI process it prints
/// "Overriding existing handler for signal 10. Set JSC_SIGNAL_FOR_GC ...". Clay hides
/// that start-up (webview_gui.rs's StderrSuppress); this is the guard that it stays
/// hidden. (Setting JSC_SIGNAL_FOR_GC is no fix: this WebKit answers any unknown JSC_*
/// variable with "ERROR: invalid option".)
#[test]
fn gui_startup_prints_no_webkit_signal_warning() {
    if Command::new("broadwayd").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        eprintln!("skipped: broadwayd (GTK's headless display server) is not installed");
        return;
    }
    let Some((_broadway, display)) = start_broadway() else {
        eprintln!("skipped: could not start broadwayd");
        return;
    };

    // A throwaway HOME, so the GUI can't touch the user's ~/.clay, with a free web port.
    let home = std::env::temp_dir().join(format!("clay-gui-startup-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".clay")).unwrap();
    std::fs::write(home.join(".clay/settings.dat"), format!("[global]\nhttp_port={}\n", free_port())).unwrap();
    let stderr_path = home.join("stderr.log");
    let stderr_file = std::fs::File::create(&stderr_path).unwrap();

    let gui = Command::new(env!("CARGO_BIN_EXE_clay"))
        .arg("--gui")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        // Only needs to be non-empty to pass Clay's display check; GTK uses Broadway.
        .env("DISPLAY", ":99")
        .env("GDK_BACKEND", "broadway")
        .env("BROADWAY_DISPLAY", format!(":{display}"))
        .env_remove("JSC_SIGNAL_FOR_GC")
        .stdout(Stdio::null())
        .stderr(stderr_file)
        .spawn()
        .expect("failed to start clay --gui");
    let mut gui = KillOnDrop(gui);

    // Long enough for the window, the webview and the page's first IPC/script calls
    // (where WebKit's lazy start-up used to print the warning).
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = gui.0.try_wait() {
            let mut err = String::new();
            let _ = std::fs::File::open(&stderr_path).and_then(|mut f| f.read_to_string(&mut err));
            let _ = std::fs::remove_dir_all(&home);
            panic!("clay --gui exited early ({status}); stderr:\n{err}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    drop(gui);

    let mut err = String::new();
    std::fs::File::open(&stderr_path).unwrap().read_to_string(&mut err).unwrap();
    let _ = std::fs::remove_dir_all(&home);
    assert!(!err.contains("Overriding existing handler for signal"),
        "the GUI printed WebKit's signal warning:\n{err}");
    assert!(!err.contains("ERROR: invalid option"),
        "the GUI printed a JavaScriptCore option error:\n{err}");
}
