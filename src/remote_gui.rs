use crate::*;
use egui::{Color32, ScrollArea, TextEdit};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};

// Windows-specific: Set title bar to dark/light mode
#[cfg(target_os = "windows")]
mod windows_titlebar {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            dw_attribute: u32,
            pv_attribute: *const c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut c_void;
        fn GetCurrentThreadId() -> u32;
        fn EnumThreadWindows(
            dw_thread_id: u32,
            lp_fn: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            l_param: isize,
        ) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn SetWindowPos(
            hwnd: *mut c_void,
            hwnd_insert_after: *mut c_void,
            x: i32, y: i32, cx: i32, cy: i32,
            flags: u32,
        ) -> i32;
        fn GetWindowTextW(hwnd: *mut c_void, lpstring: *mut u16, max_count: i32) -> i32;
    }

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    // Cache the HWND once found (stored as usize for atomicity)
    static CACHED_HWND: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn enum_callback(hwnd: *mut c_void, lparam: isize) -> i32 {
        if IsWindowVisible(hwnd) != 0 {
            // Skip helper windows with empty titles - we want the main window
            let mut title_buf = [0u16; 4];
            let len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), 4);
            if len > 0 {
                let out = &*(lparam as *const AtomicUsize);
                out.store(hwnd as usize, Ordering::Relaxed);
                return 0; // Stop enumerating
            }
        }
        1 // Continue
    }

    /// Get our window handle reliably. Tries cached value first, then
    /// GetActiveWindow, then enumerates thread windows as fallback.
    fn get_hwnd() -> *mut c_void {
        // Try cached HWND first
        let cached = CACHED_HWND.load(Ordering::Relaxed);
        if cached != 0 {
            return cached as *mut c_void;
        }

        unsafe {
            // Try GetActiveWindow (works when our window is focused)
            let hwnd = GetActiveWindow();
            if !hwnd.is_null() {
                CACHED_HWND.store(hwnd as usize, Ordering::Relaxed);
                return hwnd;
            }

            // Fallback: enumerate visible windows on our thread
            let found = AtomicUsize::new(0);
            EnumThreadWindows(
                GetCurrentThreadId(),
                enum_callback,
                &found as *const AtomicUsize as isize,
            );
            let hwnd = found.load(Ordering::Relaxed) as *mut c_void;
            if !hwnd.is_null() {
                CACHED_HWND.store(hwnd as usize, Ordering::Relaxed);
            }
            hwnd
        }
    }

    /// Set the title bar to dark or light mode, using the native Windows 11
    /// title bar colors. Returns true if the DWM call succeeded.
    pub fn apply_titlebar_theme(dark: bool) -> bool {
        let hwnd = get_hwnd();
        if hwnd.is_null() {
            return false;
        }
        unsafe {
            let value: u32 = if dark { 1 } else { 0 };
            let hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &value as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            if hr != 0 { return false; }

            // Force Windows to redraw the window frame so DWM changes take effect
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            true
        }
    }
}

/// Cached "now" time for batch timestamp formatting in GUI
struct GuiCachedNow;

impl GuiCachedNow {
    fn new() -> Self {
        Self
    }
}

/// ANSI text style attributes tracked during SGR parsing
#[derive(Clone, Default)]
struct AnsiStyle {
    bold: bool,
    dim: bool,
    italics: bool,
    underline: bool,
    blink: bool,
    reverse: bool,
    strikethrough: bool,
    /// Whether blink text is currently visible (toggled by animation timer)
    blink_visible: bool,
}

/// World settings for remote GUI
#[derive(Clone, Default)]
pub struct RemoteWorldSettings {
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub use_ssl: bool,
    pub log_enabled: bool,
    pub encoding: String,
    pub auto_login: String,
    pub keep_alive_type: String,
    pub keep_alive_cmd: String,
    pub gmcp_packages: String,
}

/// State for a remote world
#[derive(Clone)]
pub struct RemoteWorld {
    pub name: String,
    pub connected: bool,
    pub was_connected: bool,  // Whether world has ever connected (for separator bar display)
    pub is_proxy: bool,       // Whether the connection uses a TLS proxy
    pub output_lines: Vec<TimestampedLine>,
    pub prompt: String,
    pub settings: RemoteWorldSettings,
    pub unseen_lines: usize,
    pub pending_count: usize,  // Server's pending line count (for synchronized more-mode)
    // Timing info (seconds since event, None if never)
    pub last_send_secs: Option<u64>,
    pub last_recv_secs: Option<u64>,
    pub last_nop_secs: Option<u64>,
    // Partial line handling (for lines split across multiple WebSocket messages)
    pub partial_line: String,
    // Whether to show centered splash screen
    pub showing_splash: bool,
    // Whether GMCP user processing is enabled (F9 toggle)
    pub gmcp_user_enabled: bool,
}

/// Which popup is currently open
#[derive(PartialEq, Clone)]
enum PopupState {
    None,
    ConnectedWorlds,  // Combined world selector and connected worlds list
    WorldEditor(usize),  // world index being edited
    WorldConfirmDelete(usize),  // world index to delete
    Setup,
    Web,  // /web - web settings (HTTP/HTTPS/WS)
    Font,
    Help,
    Menu,  // /menu - popup to select windows
    ActionsList,           // Actions list (first window)
    ActionEditor(usize),   // Action editor (second window) - index of action being edited
    ActionConfirmDelete,   // Delete confirmation dialog
    DebugText,             // Debug popup showing raw ANSI codes
}

/// Remote GUI client application state
/// GUI Theme - mirrors the TUI Theme but with egui colors
/// GUI theme wrapper that delegates to ThemeColors from the theme file.
/// Provides Color32-returning methods for egui rendering.
#[derive(Clone)]
struct GuiTheme {
    /// The underlying theme colors from ~/clay.theme.dat
    colors: theme::ThemeColors,
    /// Whether this is a dark theme (for is_dark() checks and ANSI adjustments)
    dark: bool,
}

#[allow(dead_code)]
impl GuiTheme {
    fn from_name(name: &str) -> Self {
        let dark = name != "light";
        Self {
            colors: if dark { theme::ThemeColors::dark_default() } else { theme::ThemeColors::light_default() },
            dark,
        }
    }

    fn from_theme_colors(colors: theme::ThemeColors, dark: bool) -> Self {
        Self { colors, dark }
    }

    fn name(&self) -> &'static str {
        if self.dark { "Dark" } else { "Light" }
    }

    fn next(&self) -> Self {
        // Toggle: return the opposite theme with defaults
        // (full theme colors will be received from server via settings update)
        Self::from_name(if self.dark { "light" } else { "dark" })
    }

    fn is_dark(&self) -> bool {
        self.dark
    }

    fn to_string_value(&self) -> String {
        if self.dark { "dark".to_string() } else { "light".to_string() }
    }

    /// Update colors from JSON received via WebSocket
    fn update_from_json(&mut self, json: &str) {
        let base = if self.dark { theme::ThemeColors::dark_default() } else { theme::ThemeColors::light_default() };
        self.colors = theme::ThemeColors::from_json(json, &base);
    }

    // Helper to convert ThemeColor to egui Color32
    fn c(tc: &theme::ThemeColor) -> Color32 {
        Color32::from_rgb(tc.r, tc.g, tc.b)
    }

    // Background hierarchy
    fn bg_deep(&self) -> Color32 { Self::c(&self.colors.bg_deep) }
    fn bg(&self) -> Color32 { Self::c(&self.colors.bg) }
    fn bg_surface(&self) -> Color32 { Self::c(&self.colors.bg_surface) }
    fn bg_elevated(&self) -> Color32 { Self::c(&self.colors.bg_elevated) }
    fn bg_hover(&self) -> Color32 { Self::c(&self.colors.bg_hover) }

    // Text hierarchy
    fn fg(&self) -> Color32 { Self::c(&self.colors.fg) }
    fn fg_secondary(&self) -> Color32 { Self::c(&self.colors.fg_secondary) }
    fn fg_muted(&self) -> Color32 { Self::c(&self.colors.fg_muted) }
    fn fg_dim(&self) -> Color32 { Self::c(&self.colors.fg_dim) }

    // Accent colors
    fn accent(&self) -> Color32 { Self::c(&self.colors.accent) }
    fn accent_dim(&self) -> Color32 { Self::c(&self.colors.accent_dim) }
    fn highlight(&self) -> Color32 { Self::c(&self.colors.highlight) }
    fn success(&self) -> Color32 { Self::c(&self.colors.success) }
    fn error(&self) -> Color32 { Self::c(&self.colors.error) }
    fn error_dim(&self) -> Color32 { Self::c(&self.colors.error_dim) }

    // Borders
    fn border_subtle(&self) -> Color32 { Self::c(&self.colors.border_subtle) }
    fn border_medium(&self) -> Color32 { Self::c(&self.colors.border_medium) }

    fn panel_bg(&self) -> Color32 { self.bg() }
    fn button_bg(&self) -> Color32 { self.bg_hover() }
    fn selection_bg(&self) -> Color32 { Self::c(&self.colors.selection_bg) }
    fn prompt(&self) -> Color32 { Self::c(&self.colors.prompt) }
    fn link(&self) -> Color32 { Self::c(&self.colors.link) }
    fn list_selection_bg(&self) -> Color32 {
        let a = self.accent();
        Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 38)
    }

    // Status bar and indicators
    fn status_bar_bg(&self) -> Color32 { Self::c(&self.colors.status_bar_bg) }
    fn menu_bar_bg(&self) -> Color32 { Self::c(&self.colors.menu_bar_bg) }
    fn more_indicator_bg(&self) -> Color32 { Self::c(&self.colors.more_indicator_bg) }
    fn activity_label_bg(&self) -> Color32 { Self::c(&self.colors.activity_bg) }

    fn activity_count_bg(&self) -> Color32 {
        // Slightly different shade for count vs label
        if self.dark {
            Color32::from_rgb(212, 192, 106) // #d4c06a
        } else {
            Self::c(&self.colors.activity_bg)
        }
    }
}

/// Wrapper to mimic TextEdit output for custom text rendering
struct TextEditOutputWrapper {
    response: egui::Response,
    galley: std::sync::Arc<egui::Galley>,
    cursor_range: Option<egui::text_edit::CursorRange>,
    galley_pos: egui::Pos2,
}

pub struct RemoteGuiApp {
    /// True if running in master GUI mode (in-process App, no WebSocket)
    is_master: bool,
    /// WebSocket URL
    ws_url: String,
    /// Username for authentication (multiuser mode)
    username: String,
    /// Password for authentication
    password: String,
    /// Whether server is in multiuser mode
    multiuser_mode: bool,
    /// Whether we're connected to the server
    connected: bool,
    /// Whether we're authenticated
    authenticated: bool,
    /// Error message to display
    error_message: Option<String>,
    /// Worlds received from server
    worlds: Vec<RemoteWorld>,
    /// Currently selected world index
    current_world: usize,
    /// Input buffer for commands
    input_buffer: String,
    /// Previous input buffer length (for detecting deletes vs inserts)
    prev_input_len: usize,
    /// Temperature to skip re-converting (after user undid conversion)
    skip_temp_conversion: Option<String>,
    /// Command completion state - last partial command that was completed
    completion_prefix: String,
    /// Command completion state - index of last match used
    completion_index: usize,
    /// Channel for sending messages to WebSocket
    ws_tx: Option<mpsc::UnboundedSender<WsMessage>>,
    /// Channel for receiving messages from WebSocket
    ws_rx: Option<mpsc::UnboundedReceiver<WsMessage>>,
    /// Runtime handle for async operations
    runtime: tokio::runtime::Handle,
    /// Flag indicating password was submitted
    password_submitted: bool,
    /// Flag indicating we've attempted auto-connect for allow list
    auto_connect_attempted: bool,
    /// Time when connection was established (for allow list timeout)
    connect_time: Option<std::time::Instant>,
    /// Current popup state
    popup_state: PopupState,
    /// When true, scroll the selected item to center when rendering a popup list
    popup_scroll_to_selected: bool,
    /// Hamburger menu open state (gui2)
    hamburger_menu_open: bool,
    /// Time when hamburger menu was opened (to avoid immediate close)
    hamburger_opened_time: std::time::Instant,
    /// Selected item in menu popup
    menu_selected: usize,
    /// Selected world in world list popup
    world_list_selected: usize,
    /// Filter text for worlds popup
    connected_worlds_filter: String,
    /// Only show connected worlds toggle
    only_connected_worlds: bool,
    /// Temp fields for world editor
    edit_name: String,
    edit_hostname: String,
    edit_port: String,
    edit_user: String,
    edit_password: String,
    edit_ssl: bool,
    edit_log_enabled: bool,
    edit_encoding: Encoding,
    edit_auto_login: AutoConnectType,
    edit_keep_alive_type: KeepAliveType,
    edit_keep_alive_cmd: String,
    edit_gmcp_packages: String,
    /// Input area height in lines
    input_height: u16,
    /// Console theme (for TUI on server)
    console_theme: GuiTheme,
    /// GUI theme (local)
    theme: GuiTheme,
    /// Last theme applied to title bar (to detect changes)
    #[cfg(target_os = "windows")]
    titlebar_theme: Option<GuiTheme>,
    /// When to attempt reload reconnect (None = not reconnecting)
    reload_reconnect_at: Option<std::time::Instant>,
    /// Number of reload reconnect attempts made
    reload_reconnect_attempts: u8,
    /// Font name (empty for system default)
    font_name: String,
    /// Font size in points
    font_size: f32,
    /// Font tweak: scale factor (default 1.05)
    font_scale: f32,
    /// Font tweak: vertical offset factor (default -0.02)
    font_y_offset: f32,
    /// Font tweak: baseline offset factor (default 0.0)
    font_baseline_offset: f32,
    /// Web interface font sizes (passed through to web clients)
    web_font_size_phone: f32,
    web_font_size_tablet: f32,
    web_font_size_desktop: f32,
    /// Temp field for font editor
    edit_font_name: String,
    /// Temp field for font size editor
    edit_font_size: String,
    /// Temp fields for font tweak editor
    edit_font_scale: String,
    edit_font_y_offset: String,
    edit_font_baseline_offset: String,
    /// Last loaded font name (to avoid reloading)
    loaded_font_name: String,
    /// Last loaded font tweak values (to detect changes)
    loaded_font_scale: f32,
    loaded_font_y_offset: f32,
    loaded_font_baseline_offset: f32,
    /// Command history
    command_history: Vec<String>,
    /// Current position in command history (0 = current input, 1+ = history)
    history_index: usize,
    /// Saved input when browsing history
    saved_input: String,
    /// Manual scroll offset for output (None = auto-scroll to bottom)
    scroll_offset: Option<f32>,
    /// One-time scroll jump target (consumed after one frame of rendering)
    scroll_jump_to: Option<f32>,
    /// Maximum scroll offset (content height - viewport height)
    scroll_max_offset: f32,
    /// Show MUD tags
    show_tags: bool,
    /// Highlight lines matching action patterns
    highlight_actions: bool,
    /// More mode enabled (pause on overflow)
    more_mode: bool,
    /// Spell check enabled
    spell_check_enabled: bool,
    /// Temperature conversion enabled
    temp_convert_enabled: bool,
    /// Filter text for output
    filter_text: String,
    /// Whether filter popup is open
    filter_active: bool,
    /// WebSocket allow list (CSV of hosts that can connect without password)
    ws_allow_list: String,
    /// Web secure protocol (true = https/wss, false = http/ws)
    web_secure: bool,
    /// HTTP/HTTPS server enabled
    http_enabled: bool,
    /// HTTP/HTTPS server port
    http_port: u16,
    /// WS/WSS server enabled
    ws_enabled: bool,
    /// WS/WSS server port
    ws_port: u16,
    /// TLS certificate file path
    ws_cert_file: String,
    /// TLS key file path
    ws_key_file: String,
    /// World switching mode (Unseen First or Alphabetical)
    world_switch_mode: WorldSwitchMode,
    /// Debug logging enabled (synced from server, not used locally)
    debug_enabled: bool,
    /// Spell checker for input validation
    spell_checker: SpellChecker,
    /// Spell check state (suggestions, current word, etc.)
    spell_state: SpellState,
    /// Message about spell suggestions
    suggestion_message: Option<String>,
    /// Actions synced from server
    actions: Vec<Action>,
    /// Selected action index in actions list
    actions_selected: usize,
    /// Filter text for actions list
    actions_list_filter: String,
    /// Action editor temp fields
    edit_action_name: String,
    edit_action_world: String,
    edit_action_match_type: MatchType,
    edit_action_pattern: String,
    edit_action_command: String,
    edit_action_enabled: bool,
    edit_action_startup: bool,
    /// Action error message
    action_error: Option<String>,
    /// Debug text for showing raw ANSI codes
    debug_text: String,
    /// Whether initial settings have been received from the server
    settings_received: bool,
    /// Frame counter for waiting screen diagnostic
    frame_count: u32,
    /// Window transparency (0.0 = fully transparent, 1.0 = fully opaque)
    transparency: f32,
    /// Original transparency when setup popup opened (for cancel/revert)
    original_transparency: Option<f32>,
    /// Color offset percentage (0 = disabled, 1-100 = adjustment percentage)
    color_offset_percent: u8,
    /// Blink animation phase (true = visible, toggles every ~500ms)
    blink_visible: bool,
    /// Last time blink state was toggled
    blink_last_toggle: std::time::Instant,
    /// Whether the current output contains any blink text (avoids unnecessary rebuilds)
    has_blink_text: bool,
    /// ANSI music enabled
    ansi_music_enabled: bool,
    /// TLS proxy enabled (for connection preservation over hot reload)
    tls_proxy_enabled: bool,
    /// Custom dictionary path for spell checking
    dictionary_path: String,
    /// Audio output stream (must stay alive for audio to play)
    #[cfg(all(feature = "rodio", not(target_os = "android")))]
    audio_stream: Option<rodio::OutputStream>,
    /// Audio output stream handle for playing sounds
    #[cfg(all(feature = "rodio", not(target_os = "android")))]
    audio_stream_handle: Option<rodio::OutputStreamHandle>,
    /// Text selection start cursor (character index) per world
    selection_start: Option<usize>,
    /// Text selection end cursor (character index) per world - updated during drag
    selection_end: Option<usize>,
    /// Whether we're currently dragging a selection
    selection_dragging: bool,
    /// Approximate number of lines visible in output area (for more-mode)
    output_visible_lines: usize,
    /// Last sent view state (world_index, visible_lines) to avoid redundant messages
    last_sent_view_state: Option<(usize, usize)>,
    /// Activity count from server (number of worlds with unseen/pending output)
    server_activity_count: usize,
    /// Unified popup state for new popup system
    unified_popup: Option<crate::popup::PopupState>,
    /// Cached URL positions from galley text (computed once per frame)
    cached_urls: Vec<(usize, usize, String)>,
    /// Whether cached URLs need recomputation
    urls_dirty: bool,
    /// Whether output needs rebuilding (dirty flag for caching)
    output_dirty: bool,
    /// Cached LayoutJob for output area
    cached_output_job: Option<egui::text::LayoutJob>,
    /// Cached plain text for copy operations
    cached_plain_text: String,
    /// Cached display lines (for emoji rendering path)
    cached_display_lines: Vec<String>,
    /// Cached has_discord_emojis flag
    cached_has_emojis: bool,
    /// Last available width used for output layout
    cached_output_width: f32,
    /// Last output line count (to detect changes without explicit dirty flag)
    cached_output_len: usize,
    /// Last world index rendered (to detect world switches)
    cached_world_index: usize,
    /// Last show_tags state
    cached_show_tags: bool,
    /// Last highlight_actions state
    cached_highlight_actions: bool,
    /// Last filter text
    cached_filter_text: String,
    /// Last font size
    cached_font_size: f32,
    /// Last color_offset_percent
    cached_color_offset: u8,
}

/// Square wave audio source for ANSI music playback
#[cfg(all(feature = "rodio", not(target_os = "android")))]
struct SquareWaveSource {
    sample_rate: u32,
    frequency: f32,
    duration_samples: usize,
    current_sample: usize,
}

#[cfg(all(feature = "rodio", not(target_os = "android")))]
impl SquareWaveSource {
    fn new(frequency: f32, duration_ms: u32, sample_rate: u32) -> Self {
        let duration_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
        Self {
            sample_rate,
            frequency,
            duration_samples,
            current_sample: 0,
        }
    }
}

#[cfg(all(feature = "rodio", not(target_os = "android")))]
impl Iterator for SquareWaveSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_sample >= self.duration_samples {
            return None;
        }

        let t = self.current_sample as f32 / self.sample_rate as f32;
        let sample = if self.frequency > 0.0 {
            // Square wave: sign of sine wave
            let phase = (t * self.frequency * 2.0 * std::f32::consts::PI).sin();
            if phase >= 0.0 { 0.15 } else { -0.15 }  // Low volume to not be too loud
        } else {
            0.0  // Rest/silence
        };

        self.current_sample += 1;
        Some(sample)
    }
}

#[cfg(all(feature = "rodio", not(target_os = "android")))]
impl rodio::Source for SquareWaveSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.duration_samples - self.current_sample)
    }

    fn channels(&self) -> u16 {
        1  // Mono
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(
            (self.duration_samples as f32 / self.sample_rate as f32 * 1000.0) as u64
        ))
    }
}

/// Discord emoji segment for rendering
#[derive(Debug, Clone)]
enum DiscordSegment {
    Text(String),
    Emoji { name: String, id: String, animated: bool },
    ColoredSquare(egui::Color32),
}

impl RemoteGuiApp {
    pub fn new(ws_url: String, runtime: tokio::runtime::Handle) -> Self {
        let mut app = Self {
            is_master: false,
            ws_url,
            username: String::new(),
            password: String::new(),
            multiuser_mode: false,
            connected: false,
            authenticated: false,
            error_message: None,
            worlds: Vec::new(),
            current_world: 0,
            input_buffer: String::new(),
            prev_input_len: 0,
            skip_temp_conversion: None,
            completion_prefix: String::new(),
            completion_index: 0,
            ws_tx: None,
            ws_rx: None,
            runtime,
            password_submitted: false,
            auto_connect_attempted: false,
            connect_time: None,
            popup_state: PopupState::None,
            popup_scroll_to_selected: false,
            hamburger_menu_open: false,
            hamburger_opened_time: std::time::Instant::now(),
            menu_selected: 0,
            world_list_selected: 0,
            connected_worlds_filter: String::new(),
            only_connected_worlds: false,
            edit_name: String::new(),
            edit_hostname: String::new(),
            edit_port: String::new(),
            edit_user: String::new(),
            edit_password: String::new(),
            edit_ssl: false,
            edit_log_enabled: false,
            edit_encoding: Encoding::Utf8,
            edit_auto_login: AutoConnectType::Connect,
            edit_keep_alive_type: KeepAliveType::Nop,
            edit_keep_alive_cmd: String::new(),
            edit_gmcp_packages: String::new(),
            input_height: 3,
            console_theme: GuiTheme::from_name("dark"),
            theme: GuiTheme::from_name("dark"),
            #[cfg(target_os = "windows")]
            titlebar_theme: None,
            reload_reconnect_at: None,
            reload_reconnect_attempts: 0,
            font_name: String::new(),
            font_size: 14.0,
            font_scale: 1.05,
            font_y_offset: -0.02,
            font_baseline_offset: 0.0,
            web_font_size_phone: 10.0,
            web_font_size_tablet: 14.0,
            web_font_size_desktop: 18.0,
            edit_font_name: String::new(),
            edit_font_size: String::from("14.0"),
            edit_font_scale: String::from("1.05"),
            edit_font_y_offset: String::from("-0.02"),
            edit_font_baseline_offset: String::from("0.00"),
            loaded_font_name: String::from("__uninitialized__"),
            loaded_font_scale: 0.0,
            loaded_font_y_offset: 0.0,
            loaded_font_baseline_offset: 0.0,
            command_history: Vec::new(),
            history_index: 0,
            saved_input: String::new(),
            scroll_offset: None,
            scroll_jump_to: None,
            scroll_max_offset: 0.0,
            show_tags: false,
            highlight_actions: false,
            more_mode: true,
            spell_check_enabled: true,
            temp_convert_enabled: false,
            filter_text: String::new(),
            filter_active: false,
            ws_allow_list: String::new(),
            web_secure: false,
            http_enabled: false,
            http_port: 9000,
            ws_enabled: false,
            ws_port: 9001,
            ws_cert_file: String::new(),
            ws_key_file: String::new(),
            world_switch_mode: WorldSwitchMode::UnseenFirst,
            debug_enabled: false,
            spell_checker: SpellChecker::new(""),
            spell_state: SpellState::new(),
            suggestion_message: None,
            actions: Vec::new(),
            actions_selected: 0,
            actions_list_filter: String::new(),
            edit_action_name: String::new(),
            edit_action_world: String::new(),
            edit_action_match_type: MatchType::Regexp,
            edit_action_pattern: String::new(),
            edit_action_command: String::new(),
            edit_action_enabled: true,
            edit_action_startup: false,
            action_error: None,
            debug_text: String::new(),
            settings_received: false,
            frame_count: 0,
            transparency: 1.0,
            original_transparency: None,
            color_offset_percent: 0,
            blink_visible: true,
            blink_last_toggle: std::time::Instant::now(),
            has_blink_text: false,
            ansi_music_enabled: true,
            tls_proxy_enabled: false,
            dictionary_path: String::new(),
            #[cfg(all(feature = "rodio", not(target_os = "android")))]
            audio_stream: None,
            #[cfg(all(feature = "rodio", not(target_os = "android")))]
            audio_stream_handle: None,
            selection_start: None,
            selection_end: None,
            selection_dragging: false,
            output_visible_lines: 20,  // Default, updated during rendering
            last_sent_view_state: None,
            server_activity_count: 0,
            unified_popup: None,
            cached_urls: Vec::new(),
            urls_dirty: true,
            output_dirty: true,
            cached_output_job: None,
            cached_plain_text: String::new(),
            cached_display_lines: Vec::new(),
            cached_has_emojis: false,
            cached_output_width: 0.0,
            cached_output_len: 0,
            cached_world_index: usize::MAX,
            cached_show_tags: false,
            cached_highlight_actions: false,
            cached_filter_text: String::new(),
            cached_font_size: 0.0,
            cached_color_offset: 0,
        };
        app.load_remote_settings();
        app
    }

    /// Create a new RemoteGuiApp in master mode (in-process App, no WebSocket).
    /// The GUI starts pre-connected and pre-authenticated since it talks directly to the App.
    pub fn new_master(
        ws_rx: tokio::sync::mpsc::UnboundedReceiver<crate::WsMessage>,
        ws_tx: tokio::sync::mpsc::UnboundedSender<crate::WsMessage>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let mut app = Self::new(String::new(), runtime);
        app.is_master = true;
        app.connected = true;
        app.authenticated = true;
        app.ws_rx = Some(ws_rx);
        app.ws_tx = Some(ws_tx);
        app.auto_connect_attempted = true; // Skip WebSocket auto-connect
        app
    }

    /// Get the path for the local remote settings cache file
    fn get_remote_settings_path() -> Option<std::path::PathBuf> {
        home::home_dir().map(|p| p.join(".clay.remote.dat"))
    }

    /// Load cached settings from ~/.clay.remote.dat
    /// These are temporary settings used before the server sends the real ones.
    fn load_remote_settings(&mut self) {
        let path = match Self::get_remote_settings_path() {
            Some(p) => p,
            None => return,
        };
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "gui_theme" => self.theme = GuiTheme::from_name(value),
                    "font_size" => if let Ok(v) = value.parse::<f32>() { self.font_size = v; },
                    "font_name" => self.font_name = value.to_string(),
                    "font_scale" => if let Ok(v) = value.parse::<f32>() { self.font_scale = v; },
                    "font_y_offset" => if let Ok(v) = value.parse::<f32>() { self.font_y_offset = v; },
                    "font_baseline_offset" => if let Ok(v) = value.parse::<f32>() { self.font_baseline_offset = v; },
                    "input_height" => if let Ok(v) = value.parse::<u16>() { self.input_height = v; },
                    "transparency" => if let Ok(v) = value.parse::<f32>() { self.transparency = v; },
                    "color_offset_percent" => if let Ok(v) = value.parse::<u8>() { self.color_offset_percent = v; },
                    _ => {}
                }
            }
        }
    }

    /// Save current settings to ~/.clay.remote.dat
    /// Only saves settings that affect initial appearance before server auth.
    fn save_remote_settings(&self) {
        let path = match Self::get_remote_settings_path() {
            Some(p) => p,
            None => return,
        };
        let contents = format!(
            "# Clay remote client cached settings\n\
             # These are temporary defaults used before authenticating with the server.\n\
             # Server settings will override these once connected and authenticated.\n\
             gui_theme = {}\n\
             font_size = {}\n\
             font_name = {}\n\
             font_scale = {}\n\
             font_y_offset = {}\n\
             font_baseline_offset = {}\n\
             input_height = {}\n\
             transparency = {}\n\
             color_offset_percent = {}\n",
            self.theme.to_string_value(),
            self.font_size,
            self.font_name,
            self.font_scale,
            self.font_y_offset,
            self.font_baseline_offset,
            self.input_height,
            self.transparency,
            self.color_offset_percent,
        );
        let _ = std::fs::write(&path, contents);
    }

    /// Initialize audio output for ANSI music playback
    #[cfg(all(feature = "rodio", not(target_os = "android")))]
    fn init_audio(&mut self) {
        if self.audio_stream.is_none() {
            match rodio::OutputStream::try_default() {
                Ok((stream, handle)) => {
                    self.audio_stream = Some(stream);
                    self.audio_stream_handle = Some(handle);
                }
                Err(_) => {
                    // Audio initialization failed - music will be silently disabled
                }
            }
        }
    }

    /// Play ANSI music notes
    #[cfg(all(feature = "rodio", not(target_os = "android")))]
    fn play_ansi_music(&mut self, notes: &[crate::ansi_music::MusicNote]) {
        if !self.ansi_music_enabled || notes.is_empty() {
            return;
        }

        // Initialize audio if not already done
        self.init_audio();

        if let Some(handle) = &self.audio_stream_handle {
            // Create a sink for sequential playback
            if let Ok(sink) = rodio::Sink::try_new(handle) {
                for note in notes {
                    let source = SquareWaveSource::new(note.frequency, note.duration_ms, 44100);
                    sink.append(source);
                }
                // Detach the sink so it plays in the background
                sink.detach();
            }
        }
    }

    /// Play ANSI music notes (no-op when rodio is not available)
    #[cfg(any(not(feature = "rodio"), target_os = "android"))]
    fn play_ansi_music(&mut self, _notes: &[crate::ansi_music::MusicNote]) {
        // Audio playback disabled - rodio feature not enabled
    }

    /// Open the world selector popup
    fn open_world_selector_unified(&mut self) {
        self.popup_state = PopupState::ConnectedWorlds;
        self.world_list_selected = self.current_world;
        self.only_connected_worlds = false;
        self.popup_scroll_to_selected = true;
    }

    /// Open the actions list popup using unified system
    fn open_actions_list_unified(&mut self) {
        use crate::popup::definitions::actions::*;

        let actions: Vec<ActionInfo> = self.actions.iter()
            .enumerate()
            .map(|(i, a)| ActionInfo {
                name: a.name.clone(),
                world: a.world.clone(),
                pattern: a.pattern.clone(),
                enabled: a.enabled,
                index: i,
            })
            .collect();

        let visible_height = 10.min(actions.len().max(3));
        let def = create_actions_list_popup(&actions, visible_height);
        self.unified_popup = Some(crate::popup::PopupState::new(def));
        self.popup_scroll_to_selected = true;
    }

    /// Open the connections popup using unified system
    fn open_connections_unified(&mut self) {
        use crate::popup::definitions::connections::*;

        let connections: Vec<ConnectionInfo> = self.worlds.iter().enumerate()
            .map(|(idx, w)| {
                let last_send = w.last_send_secs.map(|s| format_elapsed(Some(s))).unwrap_or_else(|| "-".to_string());
                let last_recv = w.last_recv_secs.map(|s| format_elapsed(Some(s))).unwrap_or_else(|| "-".to_string());
                let last = format!("{}/{}", last_recv, last_send);
                let last_nop = w.last_nop_secs.map(|s| format_elapsed(Some(s))).unwrap_or_else(|| "-".to_string());
                let next_nop = format_next_nop(w.last_send_secs, w.last_recv_secs);
                let ka = format!("{}/{}", last_nop, next_nop);

                ConnectionInfo {
                    name: w.name.clone(),
                    is_current: idx == self.current_world,
                    is_connected: w.connected,
                    is_ssl: w.settings.use_ssl,
                    is_proxy: w.is_proxy,
                    unseen_lines: w.unseen_lines,
                    last,
                    ka,
                    buffer_size: w.output_lines.len(),
                }
            })
            .collect();

        let visible_height = 10.min(connections.iter().filter(|c| c.is_connected).count().max(3));
        let def = create_connections_popup(&connections, visible_height);
        self.unified_popup = Some(crate::popup::PopupState::new(def));
        self.popup_scroll_to_selected = true;
    }

    /// Try to find a system font file by name
    fn find_system_font(font_name: &str) -> Option<Vec<u8>> {
        // Common font directories on Linux
        let font_dirs = [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "~/.fonts",
            "~/.local/share/fonts",
        ];

        // Map font names to common file names
        let file_patterns: &[&str] = match font_name {
            "Monospace" => &["DejaVuSansMono.ttf", "LiberationMono-Regular.ttf", "UbuntuMono-R.ttf"],
            "DejaVu Sans Mono" => &["DejaVuSansMono.ttf"],
            "Liberation Mono" => &["LiberationMono-Regular.ttf"],
            "Ubuntu Mono" => &["UbuntuMono-R.ttf", "UbuntuMono-Regular.ttf"],
            "Fira Code" => &["FiraCode-Regular.ttf", "FiraCode-Retina.ttf"],
            "Source Code Pro" => &["SourceCodePro-Regular.ttf", "SourceCodePro-Regular.otf"],
            "JetBrains Mono" => &["JetBrainsMono-Regular.ttf", "JetBrainsMono[wght].ttf"],
            "Hack" => &["Hack-Regular.ttf"],
            "Inconsolata" => &["Inconsolata-Regular.ttf", "Inconsolata.ttf"],
            "Courier New" => &["cour.ttf", "CourierNew.ttf"],
            "Consolas" => &["consola.ttf", "Consolas.ttf"],
            _ => &[],
        };

        // Search for font files
        for dir in &font_dirs {
            let dir_path = if dir.starts_with('~') {
                if let Some(home) = std::env::var_os("HOME") {
                    std::path::PathBuf::from(home).join(&dir[2..])
                } else {
                    continue;
                }
            } else {
                std::path::PathBuf::from(dir)
            };

            if !dir_path.exists() {
                continue;
            }

            // Recursively search for font files
            fn search_dir(dir: &std::path::Path, patterns: &[&str]) -> Option<Vec<u8>> {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            if let Some(data) = search_dir(&path, patterns) {
                                return Some(data);
                            }
                        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            for pattern in patterns {
                                if name.eq_ignore_ascii_case(pattern) {
                                    if let Ok(data) = std::fs::read(&path) {
                                        return Some(data);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }

            if let Some(data) = search_dir(&dir_path, file_patterns) {
                return Some(data);
            }
        }

        None
    }

    fn connect_websocket(&mut self) {
        // Determine URL scheme - if ws_url starts with wss:// or ws://, use as-is
        // Otherwise, try wss:// (secure) by default
        let url = if self.ws_url.starts_with("ws://") || self.ws_url.starts_with("wss://") {
            self.ws_url.clone()
        } else {
            // Default to wss:// for security
            format!("wss://{}", self.ws_url)
        };

        let (tx, rx) = mpsc::unbounded_channel::<WsMessage>();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();

        self.ws_tx = Some(out_tx);
        self.ws_rx = Some(rx);

        let password_hash = hash_password(&self.password);
        let password_submitted = self.password_submitted;
        let ws_url_for_fallback = self.ws_url.clone();

        self.runtime.spawn(async move {
            // Try to connect - for wss:// we need to configure TLS
            #[cfg(feature = "native-tls-backend")]
            let connect_result = if url.starts_with("wss://") {
                // Create a TLS connector that accepts self-signed certificates
                let tls_connector = native_tls::TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .danger_accept_invalid_hostnames(true)
                    .build()
                    .map_err(|e| tokio_tungstenite::tungstenite::Error::Tls(
                        tokio_tungstenite::tungstenite::error::TlsError::Native(e)
                    ));

                match tls_connector {
                    Ok(connector) => {
                        let connector = tokio_tungstenite::Connector::NativeTls(connector);
                        tokio_tungstenite::connect_async_tls_with_config(
                            &url,
                            None,
                            false,
                            Some(connector),
                        ).await.map(|(ws, resp)| (ws, resp))
                    }
                    Err(e) => Err(e),
                }
            } else {
                connect_async(&url).await
            };

            // With rustls backend, try wss:// first then fall back to ws://
            #[cfg(not(feature = "native-tls-backend"))]
            let connect_result = {
                if url.starts_with("wss://") {
                    // Configure rustls to accept self-signed/invalid certificates
                    let tls_config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::danger::NoCertificateVerification::new()))
                        .with_no_client_auth();
                    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls_config));
                    let result = tokio_tungstenite::connect_async_tls_with_config(
                        &url,
                        None,
                        false,
                        Some(connector),
                    ).await;

                    // If wss:// failed and we defaulted to it, try ws:// as fallback
                    match result {
                        Ok(r) => Ok(r),
                        Err(e) if !ws_url_for_fallback.starts_with("wss://") => {
                            // wss:// failed, try ws:// fallback
                            let fallback_url = format!("ws://{}", ws_url_for_fallback);
                            eprintln!("wss:// connection failed ({}), trying ws://...", e);
                            connect_async(&fallback_url).await
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    // Plain ws:// connection
                    connect_async(&url).await
                }
            };

            // If wss:// failed and we defaulted to it, try ws:// as fallback
            #[cfg(feature = "native-tls-backend")]
            let connect_result = match connect_result {
                Ok(result) => Ok(result),
                Err(_e) if url.starts_with("wss://") && !ws_url_for_fallback.starts_with("wss://") => {
                    // Try ws:// fallback
                    let fallback_url = format!("ws://{}", ws_url_for_fallback);
                    connect_async(&fallback_url).await
                }
                Err(e) => Err(e),
            };

            match connect_result {
                Ok((ws_stream, _)) => {
                    use futures::SinkExt;
                    let (mut ws_sink, mut ws_source) = ws_stream.split();

                    // Send auth request if password was submitted
                    if password_submitted {
                        let auth_msg = WsMessage::AuthRequest { username: None, password_hash, current_world: None, auth_key: None, request_key: false };
                        if let Ok(json) = serde_json::to_string(&auth_msg) {
                            let _ = ws_sink.send(WsRawMessage::Text(json)).await;
                        }
                    }

                    // Spawn sender task
                    let mut ws_sink = ws_sink;
                    tokio::spawn(async move {
                        while let Some(msg) = out_rx.recv().await {
                            if let Ok(json) = serde_json::to_string(&msg) {
                                if ws_sink.send(WsRawMessage::Text(json)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    });

                    // Receive messages
                    while let Some(msg_result) = ws_source.next().await {
                        match msg_result {
                            Ok(WsRawMessage::Text(text)) => {
                                if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                    if tx.send(ws_msg).is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(WsRawMessage::Close(_)) => break,
                            Err(_) => break,
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(WsMessage::AuthResponse {
                        success: false,
                        error: Some(format!("Connection failed: {}", e)),
                        username: None,
                        multiuser_mode: false,
                    });
                }
            }
        });

        self.connected = true;
        self.connect_time = Some(std::time::Instant::now());
    }

    /// Send authentication request with current password
    fn send_auth(&mut self) {
        if let Some(ref tx) = self.ws_tx {
            let password_hash = hash_password(&self.password);
            let username = if self.multiuser_mode && !self.username.is_empty() {
                Some(self.username.clone())
            } else {
                None
            };
            let _ = tx.send(WsMessage::AuthRequest { username, password_hash, current_world: None, auth_key: None, request_key: false });
        }
    }

    fn process_messages(&mut self) -> bool {
        let mut had_messages = false;
        // Collect deferred actions to avoid borrow issues
        let mut deferred_switch: Option<usize> = None;
        let mut deferred_connect: Option<usize> = None;
        let mut deferred_edit: Option<usize> = None;
        let mut deferred_music: Vec<crate::ansi_music::MusicNote> = Vec::new();
        let mut deferred_open_actions = false;
        let deferred_open_connections = false;
        let mut deferred_save_remote = false;

        if let Some(ref mut rx) = self.ws_rx {
            while let Ok(msg) = rx.try_recv() {
                had_messages = true;
                match msg {
                    WsMessage::ServerHello { multiuser_mode } => {
                        self.multiuser_mode = multiuser_mode;
                    }
                    WsMessage::AuthResponse { success, error, .. } => {
                        if success {
                            self.authenticated = true;
                            self.error_message = None;
                            self.reload_reconnect_at = None;
                            self.reload_reconnect_attempts = 0;
                            // Declare client type to server (RemoteGUI for egui clients)
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::ClientTypeDeclaration {
                                    client_type: crate::websocket::RemoteClientType::RemoteGUI,
                                });
                            }
                        } else {
                            self.error_message = error;
                            self.authenticated = false;
                        }
                    }
                    WsMessage::InitialState { worlds, current_world_index, settings, actions, .. } => {
                        // Track if this is first InitialState or a resync
                        let is_resync = !self.worlds.is_empty();
                        self.worlds = worlds.into_iter().map(|w| {
                            // Calculate pending count (for synchronized more-mode indicator)
                            let pending_count = if !w.pending_lines_ts.is_empty() {
                                w.pending_lines_ts.len()
                            } else {
                                w.pending_lines.len()
                            };
                            RemoteWorld {
                            name: w.name,
                            connected: w.connected,
                            was_connected: w.was_connected,
                            is_proxy: w.is_proxy,
                            // For synchronized more-mode: only use output_lines, NOT pending_lines
                            // Pending lines will be sent as ServerData when released
                            output_lines: {
                                if !w.output_lines_ts.is_empty() {
                                    w.output_lines_ts
                                } else {
                                    // Fallback for old protocol: use current time
                                    let now = current_timestamp_secs();
                                    w.output_lines.into_iter().enumerate().map(|(i, text)| TimestampedLine { text, ts: now, gagged: false, from_server: true, seq: i as u64, highlight_color: None }).collect()
                                }
                            },
                            prompt: w.prompt,
                            settings: RemoteWorldSettings {
                                hostname: w.settings.hostname,
                                port: w.settings.port,
                                user: w.settings.user,
                                password: decrypt_password(&w.settings.password),
                                use_ssl: w.settings.use_ssl,
                                log_enabled: w.settings.log_enabled,
                                encoding: w.settings.encoding,
                                auto_login: w.settings.auto_connect_type,
                                keep_alive_type: w.keep_alive_type.clone(),
                                keep_alive_cmd: w.settings.keep_alive_cmd.clone(),
                                gmcp_packages: w.settings.gmcp_packages.clone(),
                            },
                            unseen_lines: w.unseen_lines,  // Use server's centralized unseen tracking
                            pending_count,
                            last_send_secs: w.last_send_secs,
                            last_recv_secs: w.last_recv_secs,
                            last_nop_secs: w.last_nop_secs,
                            partial_line: String::new(),
                            showing_splash: w.showing_splash,
                            gmcp_user_enabled: w.gmcp_user_enabled,
                        }}).collect();
                        // On first InitialState, use server's world index
                        // On resync, preserve current world (bounded by new world count)
                        if !is_resync {
                            self.current_world = current_world_index;
                            self.selection_start = None; self.selection_end = None;
                        } else if self.current_world >= self.worlds.len() {
                            self.current_world = self.worlds.len().saturating_sub(1);
                            self.selection_start = None; self.selection_end = None;
                        }
                        self.console_theme = GuiTheme::from_name(&settings.console_theme);
                        self.theme = GuiTheme::from_name(&settings.gui_theme);
                        if !settings.theme_colors_json.is_empty() {
                            self.theme.update_from_json(&settings.theme_colors_json);
                        }
                        self.settings_received = true;
                        self.font_name = settings.font_name;
                        self.font_size = settings.font_size;
                        self.web_font_size_phone = settings.web_font_size_phone;
                        self.web_font_size_tablet = settings.web_font_size_tablet;
                        self.web_font_size_desktop = settings.web_font_size_desktop;
                        self.transparency = settings.gui_transparency;
                        self.color_offset_percent = settings.color_offset_percent;
                        self.ws_allow_list = settings.ws_allow_list;
                        self.web_secure = settings.web_secure;
                        self.http_enabled = settings.http_enabled;
                        self.http_port = settings.http_port;
                        self.ws_enabled = settings.ws_enabled;
                        self.ws_port = settings.ws_port;
                        self.ws_cert_file = settings.ws_cert_file;
                        self.ws_key_file = settings.ws_key_file;
                        self.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switch_mode);
                        self.debug_enabled = settings.debug_enabled;
                        self.more_mode = settings.more_mode_enabled;
                        self.spell_check_enabled = settings.spell_check_enabled;
                        self.temp_convert_enabled = settings.temp_convert_enabled;
                        self.show_tags = settings.show_tags;
                        self.ansi_music_enabled = settings.ansi_music_enabled;
                        self.tls_proxy_enabled = settings.tls_proxy_enabled;
                        self.dictionary_path = settings.dictionary_path.clone();
                        self.actions = actions;
                        // Send initial view state for synchronized more-mode
                        if let Some(ref tx) = self.ws_tx {
                            let _ = tx.send(WsMessage::UpdateViewState {
                                world_index: self.current_world,
                                visible_lines: self.output_visible_lines,
                            });
                            self.last_sent_view_state = Some((self.current_world, self.output_visible_lines));
                        }
                        deferred_save_remote = true;
                    }
                    WsMessage::ServerData { world_index, data, is_viewed: _, ts, from_server } => {
                        if world_index < self.worlds.len() {
                            let world = &mut self.worlds[world_index];

                            // Clear splash screen when real server data arrives
                            if world.showing_splash {
                                world.showing_splash = false;
                                world.output_lines.clear();
                            }

                            // Client-generated messages (from_server: false) are always complete
                            // Only use partial line handling for MUD server data
                            let combined = if from_server && !world.partial_line.is_empty() {
                                let mut s = std::mem::take(&mut world.partial_line);
                                s.push_str(&data);
                                s
                            } else {
                                data.clone()
                            };

                            // Check if data ends with newline (complete line) or not (partial)
                            let ends_with_newline = combined.ends_with('\n');

                            // Split into lines
                            let lines: Vec<&str> = combined.lines().collect();
                            let line_count = lines.len();

                            for (i, line) in lines.into_iter().enumerate() {
                                let is_last = i == line_count - 1;

                                // Only save partial lines for server data (client messages are always complete)
                                if from_server && is_last && !ends_with_newline {
                                    // Last line without trailing newline - it's a partial
                                    world.partial_line = line.to_string();
                                } else {
                                    // Skip lines that are only ANSI codes (cursor control garbage)
                                    if is_ansi_only_line(line) {
                                        continue;
                                    }
                                    // Complete line - add to output
                                    // Truncate very long lines to prevent performance issues
                                    let text = if line.len() > MAX_LINE_LENGTH {
                                        let mut truncate_at = MAX_LINE_LENGTH;
                                        while truncate_at > 0 && !line.is_char_boundary(truncate_at) {
                                            truncate_at -= 1;
                                        }
                                        format!("{}\x1b[0m\x1b[33m... [truncated]\x1b[0m", &line[..truncate_at])
                                    } else {
                                        line.to_string()
                                    };
                                    // All lines go to output_lines - server controls more-mode via pending_count
                                    let seq = world.output_lines.len() as u64;
                                    world.output_lines.push(TimestampedLine { text, ts, gagged: false, from_server, seq, highlight_color: None });
                                }
                            }

                            // Note: Don't track unseen_lines locally - server handles centralized tracking
                            // and will broadcast UnseenUpdate/UnseenCleared when counts change
                        }
                    }
                    WsMessage::WorldConnected { world_index, name } => {
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].connected = true;
                            self.worlds[world_index].was_connected = true;
                            self.worlds[world_index].name = name;
                        }
                    }
                    WsMessage::WorldDisconnected { world_index } => {
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].connected = false;
                        }
                    }
                    WsMessage::WorldFlushed { world_index } => {
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].output_lines.clear();
                            self.worlds[world_index].pending_count = 0;
                            self.worlds[world_index].partial_line.clear();
                        }
                    }
                    WsMessage::AnsiMusic { world_index: _, notes } => {
                        // Defer playing music to avoid borrow issues
                        deferred_music.extend(notes);
                    }
                    WsMessage::WorldAdded { world } => {
                        // Skip if we already have this world locally (from our own Add)
                        let already_exists = world.index < self.worlds.len()
                            && self.worlds[world.index].name == world.name;
                        if !already_exists {
                            let pending_count = if !world.pending_lines_ts.is_empty() {
                                world.pending_lines_ts.len()
                            } else {
                                world.pending_lines.len()
                            };
                            let new_world = RemoteWorld {
                                name: world.name.clone(),
                                connected: world.connected,
                                was_connected: world.was_connected,
                                is_proxy: world.is_proxy,
                                output_lines: if !world.output_lines_ts.is_empty() {
                                    world.output_lines_ts.clone()
                                } else {
                                    let now = current_timestamp_secs();
                                    world.output_lines.iter().enumerate().map(|(i, text)| TimestampedLine {
                                        text: text.clone(), ts: now, gagged: false, from_server: true, seq: i as u64, highlight_color: None,
                                    }).collect()
                                },
                                prompt: world.prompt.clone(),
                                settings: RemoteWorldSettings {
                                    hostname: world.settings.hostname.clone(),
                                    port: world.settings.port.clone(),
                                    user: world.settings.user.clone(),
                                    password: decrypt_password(&world.settings.password),
                                    use_ssl: world.settings.use_ssl,
                                    log_enabled: world.settings.log_enabled,
                                    encoding: world.settings.encoding.clone(),
                                    auto_login: world.settings.auto_connect_type.clone(),
                                    keep_alive_type: world.keep_alive_type.clone(),
                                    keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                                    gmcp_packages: world.settings.gmcp_packages.clone(),
                                },
                                unseen_lines: world.unseen_lines,
                                pending_count,
                                last_send_secs: world.last_send_secs,
                                last_recv_secs: world.last_recv_secs,
                                last_nop_secs: world.last_nop_secs,
                                partial_line: String::new(),
                                showing_splash: world.showing_splash,
                                gmcp_user_enabled: world.gmcp_user_enabled,
                            };
                            let insert_index = world.index.min(self.worlds.len());
                            self.worlds.insert(insert_index, new_world);
                            if self.current_world >= insert_index && self.worlds.len() > 1 {
                                self.current_world += 1;
                            }
                        }
                    }
                    WsMessage::WorldCreated { world_index } => {
                        if world_index < self.worlds.len() {
                            deferred_edit = Some(world_index);
                        }
                    }
                    WsMessage::WorldRemoved { world_index } => {
                        if world_index < self.worlds.len() {
                            self.worlds.remove(world_index);
                            // Adjust current_world if needed
                            if self.current_world >= self.worlds.len() {
                                self.current_world = self.worlds.len().saturating_sub(1);
                                self.selection_start = None; self.selection_end = None;
                            } else if self.current_world > world_index {
                                self.current_world -= 1;
                                self.selection_start = None; self.selection_end = None;
                            }
                            // Adjust world_list_selected if needed
                            if self.world_list_selected >= self.worlds.len() {
                                self.world_list_selected = self.worlds.len().saturating_sub(1);
                            } else if self.world_list_selected > world_index {
                                self.world_list_selected -= 1;
                            }
                        }
                    }
                    WsMessage::WorldSwitched { new_index } => {
                        self.current_world = new_index;
                        self.selection_start = None; self.selection_end = None;
                        // Mark seen when switching to a world
                        if new_index < self.worlds.len() {
                            self.worlds[new_index].unseen_lines = 0;
                        }
                    }
                    WsMessage::PromptUpdate { world_index, prompt } => {
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].prompt = prompt;
                        }
                    }
                    WsMessage::WorldSettingsUpdated { world_index, settings, name } => {
                        // Update local world settings from server confirmation
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].name = name;
                            self.worlds[world_index].settings.hostname = settings.hostname;
                            self.worlds[world_index].settings.port = settings.port;
                            self.worlds[world_index].settings.user = settings.user;
                            self.worlds[world_index].settings.use_ssl = settings.use_ssl;
                            self.worlds[world_index].settings.keep_alive_type = settings.keep_alive_type;
                            self.worlds[world_index].settings.keep_alive_cmd = settings.keep_alive_cmd;
                            self.worlds[world_index].settings.gmcp_packages = settings.gmcp_packages;
                        }
                    }
                    WsMessage::GlobalSettingsUpdated { settings, input_height } => {
                        // Update local global settings from server confirmation
                        self.console_theme = GuiTheme::from_name(&settings.console_theme);
                        self.theme = GuiTheme::from_name(&settings.gui_theme);
                        if !settings.theme_colors_json.is_empty() {
                            self.theme.update_from_json(&settings.theme_colors_json);
                        }
                        self.input_height = input_height;
                        self.font_name = settings.font_name;
                        self.font_size = settings.font_size;
                        self.web_font_size_phone = settings.web_font_size_phone;
                        self.web_font_size_tablet = settings.web_font_size_tablet;
                        self.web_font_size_desktop = settings.web_font_size_desktop;
                        self.transparency = settings.gui_transparency;
                        self.color_offset_percent = settings.color_offset_percent;
                        self.ws_allow_list = settings.ws_allow_list;
                        self.web_secure = settings.web_secure;
                        self.http_enabled = settings.http_enabled;
                        self.http_port = settings.http_port;
                        self.ws_enabled = settings.ws_enabled;
                        self.ws_port = settings.ws_port;
                        self.ws_cert_file = settings.ws_cert_file;
                        self.ws_key_file = settings.ws_key_file;
                        self.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switch_mode);
                        self.debug_enabled = settings.debug_enabled;
                        self.more_mode = settings.more_mode_enabled;
                        self.spell_check_enabled = settings.spell_check_enabled;
                        self.temp_convert_enabled = settings.temp_convert_enabled;
                        self.show_tags = settings.show_tags;
                        self.ansi_music_enabled = settings.ansi_music_enabled;
                        self.tls_proxy_enabled = settings.tls_proxy_enabled;
                        self.dictionary_path = settings.dictionary_path.clone();
                        deferred_save_remote = true;
                    }
                    WsMessage::SetInputBuffer { text } => {
                        self.input_buffer = text;
                    }
                    WsMessage::PendingLinesUpdate { world_index, count } => {
                        // Update pending count for world
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].pending_count = count;
                        }
                    }
                    WsMessage::PendingReleased { world_index, count: _ } => {
                        // Server/another client released pending lines
                        // GUI shows all data immediately, so just log for debugging
                        // pending_count update comes via PendingLinesUpdate
                        let _ = world_index; // suppress unused warning
                    }
                    WsMessage::WorldStateResponse { world_index, pending_count, prompt, scroll_offset: _, recent_lines: _ } => {
                        // Response to RequestWorldState - update state for the world
                        if world_index < self.worlds.len() && world_index == self.current_world {
                            self.worlds[world_index].pending_count = pending_count;
                            self.worlds[world_index].prompt = prompt;
                        }
                    }
                    WsMessage::ActionsUpdated { actions } => {
                        // Update local actions from server
                        self.actions = actions;
                    }
                    WsMessage::UnseenCleared { world_index } => {
                        // Another client (console or web) has viewed this world
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].unseen_lines = 0;
                        }
                    }
                    WsMessage::UnseenUpdate { world_index, count } => {
                        // Server's unseen count changed - update our copy
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].unseen_lines = count;
                        }
                    }
                    WsMessage::ActivityUpdate { count } => {
                        // Server's activity count changed - just display it
                        self.server_activity_count = count;
                    }
                    WsMessage::ShowTagsChanged { show_tags } => {
                        // Server toggled show_tags (F2 or /tag command)
                        self.show_tags = show_tags;
                    }
                    WsMessage::GmcpUserToggled { world_index, enabled } => {
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].gmcp_user_enabled = enabled;
                        }
                    }
                    WsMessage::CalculatedWorld { index: Some(idx) } => {
                        // Server calculated the next/prev world for us
                        if idx < self.worlds.len() && idx != self.current_world {
                            self.current_world = idx;
                            self.selection_start = None; self.selection_end = None;
                            self.worlds[idx].unseen_lines = 0;
                            self.scroll_offset = None; // Reset scroll
                            // Notify server and request current state
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::MarkWorldSeen { world_index: idx });
                                let _ = tx.send(WsMessage::RequestWorldState { world_index: idx });
                            }
                        }
                    }
                    WsMessage::CalculatedWorld { index: None } => {}
                    WsMessage::ExecuteLocalCommand { command } => {
                        // Server wants us to execute a command locally (from action)
                        let parsed = parse_command(&command);
                        match parsed {
                            Command::WorldSelector => {
                                self.popup_state = PopupState::ConnectedWorlds;
                                self.world_list_selected = self.current_world;
                                self.only_connected_worlds = false;
                                self.popup_scroll_to_selected = true;
                            }
                            Command::WorldsList => {
                                // Output connected worlds list as text
                                let worlds_info: Vec<super::util::WorldListInfo> = self.worlds.iter().enumerate().map(|(idx, world)| {
                                    super::util::WorldListInfo {
                                        name: world.name.clone(),
                                        connected: world.connected,
                                        is_current: idx == self.current_world,
                                        is_ssl: world.settings.use_ssl,
                                        is_proxy: world.is_proxy,
                                        unseen_lines: world.unseen_lines,
                                        last_send_secs: world.last_send_secs,
                                        last_recv_secs: world.last_recv_secs,
                                        last_nop_secs: world.last_nop_secs,
                                        next_nop_secs: None,
                                        buffer_size: world.output_lines.len(),
                                    }
                                }).collect();
                                let output = super::util::format_worlds_list(&worlds_info);
                                let ts = super::current_timestamp_secs();
                                if self.current_world < self.worlds.len() {
                                    for line in output.lines() {
                                        let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                        self.worlds[self.current_world].output_lines.push(TimestampedLine {
                                            text: line.to_string(),
                                            ts,
                                            gagged: false,
                                            from_server: false,
                                            seq,
                                            highlight_color: None,
                                        });
                                    }
                                }
                            }
                            Command::WorldSwitch { ref name } | Command::WorldConnectNoLogin { ref name } => {
                                // Switch to world locally, connect if needed
                                if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                                    self.current_world = idx;
                                    self.selection_start = None; self.selection_end = None;
                                    deferred_switch = Some(idx);
                                    if !self.worlds[idx].connected {
                                        deferred_connect = Some(idx);
                                    }
                                }
                                // If world not found, ignore (don't send to server to avoid console switch)
                            }
                            Command::WorldEdit { ref name } => {
                                // Open editor for world (deferred to avoid borrow issues)
                                if let Some(ref name) = name {
                                    if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                                        deferred_edit = Some(idx);
                                    }
                                } else {
                                    // Edit current world
                                    deferred_edit = Some(self.current_world);
                                }
                            }
                            Command::Help => {
                                self.popup_state = PopupState::Help;
                            }
                            Command::Version => {
                                let ts = super::current_timestamp_secs();
                                if self.current_world < self.worlds.len() {
                                    let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                    self.worlds[self.current_world].output_lines.push(
                                        TimestampedLine { text: super::get_version_string(), ts, gagged: false, from_server: false, seq, highlight_color: None }
                                    );
                                }
                            }
                            Command::Menu => {
                                self.popup_state = PopupState::Menu;
                                self.menu_selected = 0;
                            }
                            Command::Setup => {
                                self.popup_state = PopupState::Setup;
                            }
                            Command::Actions { .. } => {
                                deferred_open_actions = true;
                            }
                            Command::Disconnect => {
                                // Send disconnect to server (this is safe, won't affect console's world)
                                if let Some(ref tx) = self.ws_tx {
                                    let _ = tx.send(WsMessage::DisconnectWorld { world_index: self.current_world });
                                }
                            }
                            Command::Connect { .. } => {
                                // Connect current world
                                deferred_connect = Some(self.current_world);
                            }
                            _ => {
                                // For other commands (like /send), send to server
                                // Be careful: some commands like WorldSwitch would switch console
                                // so we handle those explicitly above
                                if let Some(ref tx) = self.ws_tx {
                                    let _ = tx.send(WsMessage::SendCommand {
                                        world_index: self.current_world,
                                        command,
                                    });
                                }
                            }
                        }
                    }
                    WsMessage::BanListResponse { .. } => {
                        // Ban list received - output is already displayed via ServerData
                    }
                    WsMessage::UnbanResult { .. } => {
                        // Unban result received - output is already displayed via ServerData
                    }
                    WsMessage::WorldSwitchResult { world_index, world_name: _, pending_count, paused: _ } => {
                        // Response to CycleWorld - update local world index and state
                        if world_index < self.worlds.len() {
                            self.current_world = world_index;
                            self.selection_start = None; self.selection_end = None;
                            self.worlds[world_index].pending_count = pending_count;
                            self.worlds[world_index].unseen_lines = 0;
                            self.scroll_offset = None; // Reset scroll on world switch
                        }
                    }
                    WsMessage::OutputLines { world_index, lines, is_initial: _ } => {
                        // Batch of output lines from server
                        if world_index < self.worlds.len() {
                            for line in lines {
                                self.worlds[world_index].output_lines.push(line);
                            }
                        }
                    }
                    WsMessage::PendingCountUpdate { world_index, count } => {
                        // Periodic pending count update from server
                        if world_index < self.worlds.len() {
                            self.worlds[world_index].pending_count = count;
                        }
                    }
                    WsMessage::ScrollbackLines { world_index: _, lines: _ } => {
                        // Response to RequestScrollback (for console clients)
                        // GUI clients have full history so this is typically not needed
                    }
                    WsMessage::ServerReloading => {
                        self.reload_reconnect_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                        self.reload_reconnect_attempts = 0;
                    }
                    _ => {}
                }
            }
        }

        // Execute deferred actions after the borrow is released
        if let Some(idx) = deferred_switch {
            self.switch_world(idx);
        }
        if let Some(idx) = deferred_connect {
            self.connect_world(idx);
        }
        if let Some(idx) = deferred_edit {
            self.open_world_editor(idx);
        }
        if !deferred_music.is_empty() {
            self.play_ansi_music(&deferred_music);
        }
        if deferred_open_actions {
            self.open_actions_list_unified();
        }
        if deferred_open_connections {
            self.open_connections_unified();
        }
        if deferred_save_remote {
            self.save_remote_settings();
        }
        had_messages
    }

    fn send_command(&mut self, world_index: usize, command: String) {
        // Store in command history (avoid duplicates of last command)
        if !command.is_empty()
            && self.command_history.last().map(|s| s.as_str()) != Some(&command)
        {
            self.command_history.push(command.clone());
        }
        // Reset history navigation
        self.history_index = 0;
        self.saved_input.clear();

        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::SendCommand { world_index, command });
        }
    }

    fn switch_world(&mut self, world_index: usize) {
        // Only send MarkWorldSeen to clear unseen count on server
        // Don't send SwitchWorld - that would switch the console too
        // GUI switching is local only (same as web interface)
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::MarkWorldSeen { world_index });
            // Request current state for this world (more indicator, prompt, etc)
            let _ = tx.send(WsMessage::RequestWorldState { world_index });
        }
    }

    fn connect_world(&mut self, world_index: usize) {
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::ConnectWorld { world_index });
        }
    }

    fn disconnect_world(&mut self, world_index: usize) {
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::DisconnectWorld { world_index });
        }
    }

    /// Find all misspelled words in the input buffer (excluding word at cursor)
    fn find_misspelled_words(&self) -> Vec<(usize, usize)> {
        let mut misspelled = Vec::new();
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut i = 0;

        // Helper to check if a character at position is part of a word
        // (alphabetic, or apostrophe between alphabetic characters)
        let is_word_char = |pos: usize| -> bool {
            if pos >= chars.len() {
                return false;
            }
            let c = chars[pos];
            if c.is_alphabetic() {
                return true;
            }
            // Include apostrophe if between alphabetic characters (contractions)
            if c == '\'' {
                let has_alpha_before = pos > 0 && chars[pos - 1].is_alphabetic();
                let has_alpha_after = pos + 1 < chars.len() && chars[pos + 1].is_alphabetic();
                return has_alpha_before && has_alpha_after;
            }
            false
        };

        // Simple cursor position estimate (egui doesn't expose cursor position easily)
        // We'll just check all words since we can't know where the cursor is during input_mut
        let cursor_char_pos = chars.len(); // Assume cursor at end for now

        // Helper to check if a word is clearly finished (followed by space or clear punctuation)
        let is_word_complete = |end_pos: usize| -> bool {
            if end_pos >= chars.len() {
                // Word at end of input - NOT complete if cursor is right at the end (still typing)
                return cursor_char_pos != end_pos;
            }
            let next_char = chars[end_pos];
            // Word is complete if followed by whitespace or clear punctuation
            next_char.is_whitespace() || matches!(next_char, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"')
        };

        while i < chars.len() {
            // Skip non-word characters
            while i < chars.len() && !chars[i].is_alphabetic() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            let start = i;
            // Continue while we have word characters (including internal apostrophes)
            while i < chars.len() && is_word_char(i) {
                i += 1;
            }
            let end = i;

            let word: String = chars[start..end].iter().collect();
            // Don't check if cursor is inside the word (actively typing)
            // Note: end is the first position after the word, so use < not <=
            let cursor_in_word = cursor_char_pos >= start && cursor_char_pos < end;
            // Only check words that are clearly complete (followed by separator or at end of input)
            let word_complete = is_word_complete(end);

            if !cursor_in_word && word_complete && !self.spell_checker.is_valid(&word) {
                misspelled.push((start, end));
            }
        }

        misspelled
    }

    /// Check for temperature patterns and convert them when followed by a separator.
    /// Patterns: 32F, 32f, 100C, 100c, 32°F, 32.5F, -10C, etc.
    /// When detected, inserts conversion in parentheses: "32F " -> "32F(0C) "
    /// Returns true if a conversion was performed (caller should update cursor to end).
    fn check_temp_conversion(&mut self) -> bool {
        // Only convert temperatures when enabled
        if !self.temp_convert_enabled {
            return false;
        }

        let current_len = self.input_buffer.len();
        // Don't convert when user is deleting - allows undoing conversion
        if current_len <= self.prev_input_len {
            self.prev_input_len = current_len;
            return false;
        }
        self.prev_input_len = current_len;

        let chars: Vec<char> = self.input_buffer.chars().collect();
        if chars.is_empty() {
            return false;
        }

        // Check if we just typed a separator after a temperature
        let last_char = chars[chars.len() - 1];
        if !last_char.is_whitespace() && !matches!(last_char, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}') {
            // Non-separator typed - clear skip so next temperature can convert
            self.skip_temp_conversion = None;
            return false;
        }

        // Look backwards for a temperature pattern before the separator
        // Pattern: optional minus, digits, optional decimal+digits, optional °, F or C
        let end = chars.len() - 1; // Position of the separator
        if end == 0 {
            return false;
        }

        // Find the F/C unit character
        let unit_pos = end - 1;
        let unit_char = chars[unit_pos].to_ascii_uppercase();
        if unit_char != 'F' && unit_char != 'C' {
            return false;
        }

        // Check for optional degree symbol before the unit
        let mut num_end = unit_pos;
        if num_end > 0 && chars[num_end - 1] == '°' {
            num_end -= 1;
        }

        // Find the start of the number (digits, optional decimal, optional leading minus)
        let mut num_start = num_end;
        let mut found_digit = false;
        let mut found_decimal = false;

        while num_start > 0 {
            let c = chars[num_start - 1];
            if c.is_ascii_digit() {
                found_digit = true;
                num_start -= 1;
            } else if c == '.' && !found_decimal {
                found_decimal = true;
                num_start -= 1;
            } else if c == '-' {
                // Allow minus at the very start of the number
                num_start -= 1;
                break;
            } else {
                break;
            }
        }

        // Check we have at least one digit
        if !found_digit {
            return false;
        }

        // Make sure the character before the number isn't part of the "word"
        // (e.g., "abc32F" shouldn't trigger, but "test 32F" should)
        if num_start > 0 {
            let prev_char = chars[num_start - 1];
            if prev_char.is_alphanumeric() || prev_char == '_' {
                return false;
            }
        }

        // Build the full temperature string (e.g., "21F", "-5.5°C")
        let temp_str: String = chars[num_start..=unit_pos].iter().collect();

        // Check if this temperature was already converted and undone - skip if so
        if let Some(ref skip) = self.skip_temp_conversion {
            if skip == &temp_str {
                return false;
            }
        }

        // Parse the number
        let num_str: String = chars[num_start..num_end].iter().collect();
        let temp: f64 = match num_str.parse() {
            Ok(t) => t,
            Err(_) => return false,
        };

        // Convert temperature
        let (converted, converted_unit) = if unit_char == 'F' {
            // Fahrenheit to Celsius: (F - 32) * 5/9
            ((temp - 32.0) * 5.0 / 9.0, 'C')
        } else {
            // Celsius to Fahrenheit: C * 9/5 + 32
            (temp * 9.0 / 5.0 + 32.0, 'F')
        };

        // Format the conversion - use integer if whole number, else one decimal
        // No space before the parenthesis - the separator the user typed goes after
        let converted_str = if (converted - converted.round()).abs() < 0.05 {
            format!("({:.0}{})", converted, converted_unit)
        } else {
            format!("({:.1}{})", converted, converted_unit)
        };

        // Remember this temperature so we don't re-convert if user undoes it
        self.skip_temp_conversion = Some(temp_str);

        // Insert the conversion before the separator
        // Build new buffer: [before separator] + conversion + [separator]
        let before_sep: String = chars[..end].iter().collect();
        let sep: String = chars[end..].iter().collect();
        self.input_buffer = format!("{}{}{}", before_sep, converted_str, sep);
        // Update prev_input_len to reflect new length after conversion
        self.prev_input_len = self.input_buffer.len();
        true
    }

    /// Get the word at or before the cursor position
    fn current_word(&self) -> Option<(usize, usize, String)> {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        if chars.is_empty() {
            return None;
        }

        // Helper to check if a character at position is part of a word
        let is_word_char = |pos: usize| -> bool {
            if pos >= chars.len() {
                return false;
            }
            let c = chars[pos];
            if c.is_alphabetic() {
                return true;
            }
            // Include apostrophe if between alphabetic characters
            if c == '\'' {
                let has_alpha_before = pos > 0 && chars[pos - 1].is_alphabetic();
                let has_alpha_after = pos + 1 < chars.len() && chars[pos + 1].is_alphabetic();
                return has_alpha_before && has_alpha_after;
            }
            false
        };

        // Assume cursor is at end of input (egui limitation)
        let cursor_pos = chars.len();
        if cursor_pos == 0 {
            return None;
        }

        // Find word boundaries around cursor
        let mut start = cursor_pos.saturating_sub(1);
        while start > 0 && is_word_char(start - 1) {
            start -= 1;
        }

        let mut end = cursor_pos;

        // If cursor is on a non-word character (e.g., space after word),
        // look backwards to find the previous word
        if !chars[start].is_alphabetic() {
            // Look back to find the last alphabetic character
            let mut prev_end = start;
            while prev_end > 0 && !chars[prev_end - 1].is_alphabetic() {
                prev_end -= 1;
            }
            if prev_end == 0 && (chars.is_empty() || !chars[0].is_alphabetic()) {
                return None;
            }
            // Now find the start of this word
            end = prev_end;
            start = prev_end;
            while start > 0 && is_word_char(start - 1) {
                start -= 1;
            }
        } else {
            while end < chars.len() && is_word_char(end) {
                end += 1;
            }
        }

        let word: String = chars[start..end].iter().collect();
        if word.is_empty() {
            return None;
        }

        Some((start, end, word))
    }

    /// Handle Ctrl+Q spell check
    fn handle_spell_check(&mut self) -> Option<String> {
        if !self.spell_state.showing_suggestions {
            // First press - find misspelled word and show suggestions
            if let Some((start, end, word)) = self.current_word() {
                if !self.spell_checker.is_valid(&word) {
                    let mut suggestions = self.spell_checker.suggestions(&word, 6);
                    if !suggestions.is_empty() {
                        self.spell_state.original_word = word.clone();
                        suggestions.push(word); // Add original at end for cycling back

                        let display_suggestions: Vec<_> = suggestions[..suggestions.len()-1].to_vec();
                        let message = format!(
                            "Suggestions for '{}': {}",
                            self.spell_state.original_word,
                            display_suggestions.join(", ")
                        );

                        self.spell_state.suggestions = suggestions;
                        self.spell_state.suggestion_index = 0;
                        self.spell_state.word_start = start;
                        self.spell_state.word_end = end;
                        self.spell_state.showing_suggestions = true;
                        self.suggestion_message = Some(format!(
                            "Press Ctrl+Q to cycle: {}",
                            self.spell_state.suggestions[0]
                        ));
                        return Some(message);
                    }
                }
            }
        } else if !self.spell_state.suggestions.is_empty() {
            // Subsequent press - cycle and apply suggestions
            let replacement = self.spell_state.suggestions[self.spell_state.suggestion_index].clone();

            // Replace word in input buffer
            let chars: Vec<char> = self.input_buffer.chars().collect();
            let before: String = chars[..self.spell_state.word_start].iter().collect();
            let after: String = if self.spell_state.word_end < chars.len() {
                chars[self.spell_state.word_end..].iter().collect()
            } else {
                String::new()
            };
            self.input_buffer = format!("{}{}{}", before, replacement, after);

            // Update word end position
            self.spell_state.word_end = self.spell_state.word_start + replacement.chars().count();
            self.spell_state.suggestion_index =
                (self.spell_state.suggestion_index + 1) % self.spell_state.suggestions.len();

            let next_word = &self.spell_state.suggestions[self.spell_state.suggestion_index];
            if next_word == &self.spell_state.original_word {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}' (original)",
                    replacement, next_word
                ));
            } else {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}'",
                    replacement, next_word
                ));
            }
        }
        None
    }

    /// Reset spell state when cursor moves away from word
    fn reset_spell_state(&mut self) {
        self.spell_state.reset();
        self.suggestion_message = None;
    }

    fn update_world_settings(&mut self, world_index: usize) {
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::UpdateWorldSettings {
                world_index,
                name: self.edit_name.clone(),
                hostname: self.edit_hostname.clone(),
                port: self.edit_port.clone(),
                user: self.edit_user.clone(),
                password: self.edit_password.clone(),
                use_ssl: self.edit_ssl,
                log_enabled: self.edit_log_enabled,
                encoding: self.edit_encoding.name().to_string(),
                auto_login: self.edit_auto_login.name().to_string(),
                keep_alive_type: self.edit_keep_alive_type.name().to_string(),
                keep_alive_cmd: self.edit_keep_alive_cmd.clone(),
                gmcp_packages: self.edit_gmcp_packages.clone(),
            });
        }
    }

    fn update_global_settings(&mut self) {
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::UpdateGlobalSettings {
                more_mode_enabled: self.more_mode,
                spell_check_enabled: self.spell_check_enabled,
                temp_convert_enabled: self.temp_convert_enabled,
                world_switch_mode: self.world_switch_mode.name().to_string(),
                show_tags: self.show_tags,
                debug_enabled: self.debug_enabled,
                ansi_music_enabled: self.ansi_music_enabled,
                console_theme: self.console_theme.to_string_value(),
                gui_theme: self.theme.to_string_value(),
                gui_transparency: self.transparency,
                color_offset_percent: self.color_offset_percent,
                input_height: self.input_height,
                font_name: self.font_name.clone(),
                font_size: self.font_size,
                web_font_size_phone: self.web_font_size_phone,
                web_font_size_tablet: self.web_font_size_tablet,
                web_font_size_desktop: self.web_font_size_desktop,
                ws_allow_list: self.ws_allow_list.clone(),
                web_secure: self.web_secure,
                http_enabled: self.http_enabled,
                http_port: self.http_port,
                ws_enabled: self.ws_enabled,
                ws_port: self.ws_port,
                ws_cert_file: self.ws_cert_file.clone(),
                ws_key_file: self.ws_key_file.clone(),
                tls_proxy_enabled: self.tls_proxy_enabled,
                dictionary_path: self.dictionary_path.clone(),
            });
        }
    }

    fn update_actions(&mut self) {
        if let Some(ref tx) = self.ws_tx {
            let _ = tx.send(WsMessage::UpdateActions {
                actions: self.actions.clone(),
            });
        }
    }

    fn open_world_editor(&mut self, world_index: usize) {
        if let Some(world) = self.worlds.get(world_index) {
            self.edit_name = world.name.clone();
            self.edit_hostname = world.settings.hostname.clone();
            self.edit_port = world.settings.port.clone();
            self.edit_user = world.settings.user.clone();
            self.edit_password = world.settings.password.clone();
            self.edit_ssl = world.settings.use_ssl;
            self.edit_log_enabled = world.settings.log_enabled;
            self.edit_encoding = match world.settings.encoding.as_str() {
                "latin1" => Encoding::Latin1,
                "fansi" => Encoding::Fansi,
                _ => Encoding::Utf8,
            };
            self.edit_auto_login = AutoConnectType::from_name(&world.settings.auto_login);
            self.edit_keep_alive_type = KeepAliveType::from_name(&world.settings.keep_alive_type);
            self.edit_keep_alive_cmd = world.settings.keep_alive_cmd.clone();
            self.edit_gmcp_packages = world.settings.gmcp_packages.clone();
            self.popup_state = PopupState::WorldEditor(world_index);
        }
    }

    /// Convert a color name to ANSI background color code (for /highlight command)
    fn color_name_to_ansi_bg(color: &str) -> String {
        let color_lower = color.to_lowercase();
        let color_lower = color_lower.trim();

        // Empty color means use default highlight
        if color_lower.is_empty() {
            return "\x1b[48;5;23m".to_string(); // Dark cyan background
        }

        // Named colors (using darker/muted versions for backgrounds)
        #[allow(clippy::redundant_slicing)]
        match &color_lower[..] {
            "red" => "\x1b[48;5;52m".to_string(),
            "green" => "\x1b[48;5;22m".to_string(),
            "blue" => "\x1b[48;5;17m".to_string(),
            "yellow" => "\x1b[48;5;58m".to_string(),
            "cyan" => "\x1b[48;5;23m".to_string(),
            "magenta" | "purple" => "\x1b[48;5;53m".to_string(),
            "orange" => "\x1b[48;5;94m".to_string(),
            "pink" => "\x1b[48;5;125m".to_string(),
            "white" => "\x1b[48;5;250m".to_string(),
            "black" => "\x1b[48;5;234m".to_string(),
            "gray" | "grey" => "\x1b[48;5;240m".to_string(),
            _ => {
                // Try parsing as xterm 256 color number
                if let Ok(num) = color_lower.parse::<u8>() {
                    return format!("\x1b[48;5;{}m", num);
                }
                // Try parsing as RGB (format: r,g,b or r;g;b)
                let parts: Vec<&str> = if color_lower.contains(',') {
                    color_lower.split(',').collect()
                } else if color_lower.contains(';') {
                    color_lower.split(';').collect()
                } else {
                    vec![]
                };
                if parts.len() == 3 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        parts[0].trim().parse::<u8>(),
                        parts[1].trim().parse::<u8>(),
                        parts[2].trim().parse::<u8>(),
                    ) {
                        return format!("\x1b[48;2;{};{};{}m", r, g, b);
                    }
                }
                // Default fallback
                "\x1b[48;5;23m".to_string() // Dark cyan
            }
        }
    }

    /// Format timestamp for GUI display
    /// Same day: HH:MM>
    /// Previous days: DD/MM HH:MM>
    #[allow(dead_code)]
    fn format_timestamp_gui(ts: u64) -> String {
        Self::format_timestamp_gui_cached(ts, &GuiCachedNow::new())
    }

    /// Format timestamp using cached "now" value for batch rendering
    fn format_timestamp_gui_cached(ts: u64, _now: &GuiCachedNow) -> String {
        let lt = local_time_from_epoch(ts as i64);

        // Always show day/month for debugging ordering issues
        format!("{:02}/{:02} {:02}:{:02}>", lt.day, lt.month, lt.hour, lt.minute)
    }

    /// Strip MUD tags like [channel:] or [channel(player)] from start of line
    fn strip_mud_tags(text: &str) -> String {
        let trimmed = text.trim_start();
        if trimmed.starts_with('[') {
            // Find the closing bracket
            if let Some(end) = trimmed.find(']') {
                let tag = &trimmed[1..end];
                // Check if it looks like a MUD tag (contains : or parentheses)
                if tag.contains(':') || tag.contains('(') {
                    // Return the rest of the line, preserving original leading whitespace
                    let leading_ws = text.len() - trimmed.len();
                    let after_tag = &trimmed[end + 1..];
                    // Trim one space after tag if present
                    let after_tag = after_tag.strip_prefix(' ').unwrap_or(after_tag);
                    return format!("{}{}", &text[..leading_ws], after_tag);
                }
            }
        }
        text.to_string()
    }

    /// Strip MUD tags from ANSI text while preserving color codes
    fn strip_mud_tags_ansi(text: &str) -> String {
        // First, find any leading whitespace
        let trimmed = text.trim_start();
        let leading_ws_len = text.len() - trimmed.len();
        let leading_ws = &text[..leading_ws_len];

        // Check if line starts with [ (possibly after ANSI codes)
        // Need to skip ANSI codes to find the actual start
        let mut chars = trimmed.chars().peekable();
        let mut ansi_prefix = String::new();
        let mut in_ansi = false;

        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                ansi_prefix.push(c);
                in_ansi = true;
            } else if in_ansi {
                ansi_prefix.push(c);
                if c.is_ascii_alphabetic() {
                    in_ansi = false;
                }
            } else if c == '[' {
                // Found the start of a potential tag
                // Look for closing bracket
                let rest: String = chars.collect();
                if let Some(end) = rest.find(']') {
                    let tag = &rest[..end];
                    if tag.contains(':') || tag.contains('(') {
                        // It's a MUD tag, skip it
                        let after_tag = &rest[end + 1..];
                        let after_tag = after_tag.strip_prefix(' ').unwrap_or(after_tag);
                        return format!("{}{}{}", leading_ws, ansi_prefix, after_tag);
                    } else {
                        // Not a MUD tag, return original
                        return text.to_string();
                    }
                } else {
                    return text.to_string();
                }
            } else {
                // Not a tag start, return original
                return text.to_string();
            }
        }
        text.to_string()
    }

    /// Convert 256-color palette index to RGB
    fn color256_to_rgb(n: u8, is_light_theme: bool) -> (u8, u8, u8) {
        match n {
            // Standard colors (0-7) - Xubuntu Dark palette
            0 => (0, 0, 0),           // Black #000000
            1 => (170, 0, 0),         // Red #aa0000
            2 => (68, 170, 68),       // Green #44aa44
            3 => if is_light_theme { (128, 64, 0) } else { (170, 85, 0) },  // Yellow #aa5500
            4 => (0, 57, 170),        // Blue #0039aa
            5 => (170, 34, 170),      // Magenta #aa22aa
            6 => (26, 146, 170),      // Cyan #1a92aa
            7 => if is_light_theme { (80, 80, 80) } else { (170, 170, 170) }, // White #aaaaaa
            // High-intensity colors (8-15) - Xubuntu Dark palette
            8 => (119, 119, 119),     // Bright Black #777777
            9 => (255, 135, 135),     // Bright Red #ff8787
            10 => (76, 230, 76),      // Bright Green #4ce64c
            11 => if is_light_theme { (167, 163, 33) } else { (222, 216, 44) }, // Bright Yellow #ded82c
            12 => (41, 95, 204),      // Bright Blue #295fcc
            13 => (204, 88, 204),     // Bright Magenta #cc58cc
            14 => (76, 204, 230),     // Bright Cyan #4ccce6
            15 => if is_light_theme { (40, 40, 40) } else { (255, 255, 255) }, // Bright White #ffffff
            // 216 colors (16-231): 6x6x6 color cube
            // Standard xterm palette uses: 0, 95, 135, 175, 215, 255
            16..=231 => {
                const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];
                let n = n - 16;
                let r = CUBE_VALUES[((n / 36) % 6) as usize];
                let g = CUBE_VALUES[((n / 6) % 6) as usize];
                let b = CUBE_VALUES[(n % 6) as usize];
                (r, g, b)
            }
            // Grayscale (232-255): 24 shades
            232..=255 => {
                let gray = 8 + (n - 232) * 10;
                (gray, gray, gray)
            }
        }
    }

    /// Blend two colors with a given weight (0.0 = all bg, 1.0 = all fg)
    fn blend_colors(fg: egui::Color32, bg: egui::Color32, fg_weight: f32) -> egui::Color32 {
        let bg_weight = 1.0 - fg_weight;
        egui::Color32::from_rgb(
            (fg.r() as f32 * fg_weight + bg.r() as f32 * bg_weight).round() as u8,
            (fg.g() as f32 * fg_weight + bg.g() as f32 * bg_weight).round() as u8,
            (fg.b() as f32 * fg_weight + bg.b() as f32 * bg_weight).round() as u8,
        )
    }

    /// Adjust foreground color for contrast when it's too similar to background.
    /// color_offset_percent: 0 = disabled, 1-100 = threshold and adjustment percentage
    fn adjust_fg_for_contrast(
        fg: egui::Color32,
        bg: egui::Color32,
        theme_bg: egui::Color32,
        color_offset_percent: u8,
    ) -> egui::Color32 {
        if color_offset_percent == 0 {
            return fg;
        }

        // Calculate effective background (use theme_bg if transparent)
        let effective_bg = if bg == egui::Color32::TRANSPARENT {
            theme_bg
        } else {
            bg
        };

        // Calculate color distance (simple RGB distance)
        let dr = (fg.r() as i32 - effective_bg.r() as i32).abs();
        let dg = (fg.g() as i32 - effective_bg.g() as i32).abs();
        let db = (fg.b() as i32 - effective_bg.b() as i32).abs();
        let distance = dr + dg + db;

        // Threshold for "too similar" - scale by color_offset_percent
        // At 100%, colors within distance 150 are adjusted
        let threshold = (150 * color_offset_percent as i32) / 100;

        if distance >= threshold {
            return fg; // Colors are different enough
        }

        // Calculate background brightness to determine if bg is light or dark
        let bg_brightness = (effective_bg.r() as u32 + effective_bg.g() as u32 + effective_bg.b() as u32) / 3;
        let is_bg_dark = bg_brightness < 128;

        // Adjustment amount based on color_offset_percent
        let adjustment = (color_offset_percent as i32 * 2).min(200); // Max 200 adjustment

        // If background is dark, lighten foreground; if light, darken foreground
        if is_bg_dark {
            egui::Color32::from_rgb(
                (fg.r() as i32 + adjustment).min(255) as u8,
                (fg.g() as i32 + adjustment).min(255) as u8,
                (fg.b() as i32 + adjustment).min(255) as u8,
            )
        } else {
            egui::Color32::from_rgb(
                (fg.r() as i32 - adjustment).max(0) as u8,
                (fg.g() as i32 - adjustment).max(0) as u8,
                (fg.b() as i32 - adjustment).max(0) as u8,
            )
        }
    }

    /// Build an egui::TextFormat with the current ANSI style flags applied
    fn make_text_format(
        font_id: &egui::FontId,
        fg_color: egui::Color32,
        bg_color: egui::Color32,
        theme_bg: egui::Color32,
        style: &AnsiStyle,
    ) -> egui::TextFormat {
        // Apply dim: reduce brightness by 50%
        let fg_color = if style.dim {
            let [r, g, b, a] = fg_color.to_array();
            egui::Color32::from_rgba_premultiplied(r / 2, g / 2, b / 2, a)
        } else {
            fg_color
        };

        // Apply reverse: swap fg and bg
        let (fg_color, bg_color) = if style.reverse {
            let effective_bg = if bg_color == egui::Color32::TRANSPARENT {
                theme_bg
            } else {
                bg_color
            };
            (effective_bg, fg_color)
        } else {
            (fg_color, bg_color)
        };

        // Apply blink: hide text during off phase
        let fg_color = if style.blink && !style.blink_visible {
            bg_color // text becomes invisible (same as background)
        } else {
            fg_color
        };

        let underline = if style.underline {
            egui::Stroke::new(1.0, fg_color)
        } else {
            egui::Stroke::NONE
        };

        let strikethrough = if style.strikethrough {
            egui::Stroke::new(1.0, fg_color)
        } else {
            egui::Stroke::NONE
        };

        egui::TextFormat {
            font_id: font_id.clone(),
            color: fg_color,
            background: bg_color,
            italics: style.italics,
            underline,
            strikethrough,
            ..Default::default()
        }
    }

    /// Append a segment to job, processing shade characters for proper color blending
    fn append_segment_with_shades(
        segment: &str,
        font_id: &egui::FontId,
        fg_color: egui::Color32,
        bg_color: egui::Color32,
        theme_bg: egui::Color32,
        color_offset_percent: u8,
        style: &AnsiStyle,
        job: &mut egui::text::LayoutJob,
    ) {
        // Apply color contrast adjustment if enabled
        let fg_color = Self::adjust_fg_for_contrast(fg_color, bg_color, theme_bg, color_offset_percent);

        // If no shade characters, just append normally
        if !segment.chars().any(|c| c == '░' || c == '▒' || c == '▓') {
            job.append(segment, 0.0, Self::make_text_format(font_id, fg_color, bg_color, theme_bg, style));
            return;
        }

        // Use explicit background if set, otherwise use theme background for blending
        let blend_bg = if bg_color == egui::Color32::TRANSPARENT {
            theme_bg
        } else {
            bg_color
        };

        // Process shade characters - group consecutive chars by their type
        let mut current_run = String::new();
        let mut current_is_shade: Option<char> = None;

        for c in segment.chars() {
            let shade_type = match c {
                '░' | '▒' | '▓' => Some(c),
                _ => None,
            };

            if shade_type != current_is_shade && !current_run.is_empty() {
                // Flush current run
                if let Some(shade_char) = current_is_shade {
                    let blended_color = match shade_char {
                        '░' => Self::blend_colors(fg_color, blend_bg, 0.25),
                        '▒' => Self::blend_colors(fg_color, blend_bg, 0.50),
                        '▓' => Self::blend_colors(fg_color, blend_bg, 0.75),
                        _ => fg_color,
                    };
                    let spaces: String = current_run.chars().map(|_| ' ').collect();
                    job.append(&spaces, 0.0, egui::TextFormat {
                        font_id: font_id.clone(),
                        color: blended_color,
                        background: blended_color,
                        ..Default::default()
                    });
                } else {
                    job.append(&current_run, 0.0, Self::make_text_format(font_id, fg_color, bg_color, theme_bg, style));
                }
                current_run.clear();
            }

            current_run.push(c);
            current_is_shade = shade_type;
        }

        // Flush final run
        if !current_run.is_empty() {
            if let Some(shade_char) = current_is_shade {
                let blended_color = match shade_char {
                    '░' => Self::blend_colors(fg_color, blend_bg, 0.25),
                    '▒' => Self::blend_colors(fg_color, blend_bg, 0.50),
                    '▓' => Self::blend_colors(fg_color, blend_bg, 0.75),
                    _ => fg_color,
                };
                let spaces: String = current_run.chars().map(|_| ' ').collect();
                job.append(&spaces, 0.0, egui::TextFormat {
                    font_id: font_id.clone(),
                    color: blended_color,
                    background: blended_color,
                    ..Default::default()
                });
            } else {
                job.append(&current_run, 0.0, Self::make_text_format(font_id, fg_color, bg_color, theme_bg, style));
            }
        }
    }

    /// Append ANSI-colored text to an existing LayoutJob
    fn append_ansi_to_job(text: &str, default_color: egui::Color32, font_id: egui::FontId, job: &mut egui::text::LayoutJob, is_light_theme: bool, color_offset_percent: u8, blink_visible: bool) {
        // Theme background for shade character blending
        let theme_bg = if is_light_theme {
            egui::Color32::from_rgb(255, 255, 255)
        } else {
            egui::Color32::from_rgb(13, 17, 23)  // Dark theme background
        };

        let mut current_color = default_color;
        let mut current_bg = egui::Color32::TRANSPARENT;
        let mut style = AnsiStyle { blink_visible, ..Default::default() };
        let mut chars = text.chars().peekable();
        let mut segment = String::new();

        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                // Flush current segment
                if !segment.is_empty() {
                    Self::append_segment_with_shades(&segment, &font_id, current_color, current_bg, theme_bg, color_offset_percent, &style, job);
                    segment.clear();
                }

                // Parse escape sequence
                chars.next(); // consume '['
                let mut code = String::new();
                let mut terminator = ' ';
                while let Some(&sc) = chars.peek() {
                    if sc.is_ascii_alphabetic() || sc == '@' || sc == '`' || sc == '~' {
                        terminator = sc;
                        chars.next();
                        break;
                    }
                    chars.next();
                    code.push(sc);
                }

                // Only parse SGR codes (sequences ending in 'm')
                // Skip other CSI sequences (cursor movement, screen clearing, etc.)
                if terminator != 'm' {
                    continue;
                }

                // Parse SGR codes (semicolon-separated)
                let parts: Vec<&str> = code.split(';').collect();
                let mut i = 0;
                while i < parts.len() {
                    match parts[i].parse::<u8>().unwrap_or(0) {
                        0 => { current_color = default_color; current_bg = egui::Color32::TRANSPARENT; style.bold = false; style.dim = false; style.italics = false; style.underline = false; style.blink = false; style.reverse = false; style.strikethrough = false; }
                        1 => style.bold = true,
                        2 => style.dim = true,
                        3 => style.italics = true,
                        4 => style.underline = true,
                        5 | 6 => style.blink = true,
                        7 => style.reverse = true,
                        9 => style.strikethrough = true,
                        22 => { style.bold = false; style.dim = false; }
                        23 => style.italics = false,
                        24 => style.underline = false,
                        25 => style.blink = false,
                        27 => style.reverse = false,
                        29 => style.strikethrough = false,
                        // Standard foreground colors (30-37) - Xubuntu Dark palette
                        // When bold is active, upgrade to bright variants (90-97)
                        30 => current_color = if style.bold {
                            egui::Color32::from_rgb(119, 119, 119)  // Bright Black #777777
                        } else {
                            egui::Color32::from_rgb(0, 0, 0)        // Black #000000
                        },
                        31 => current_color = if style.bold {
                            egui::Color32::from_rgb(255, 135, 135)  // Bright Red #ff8787
                        } else {
                            egui::Color32::from_rgb(170, 0, 0)      // Red #aa0000
                        },
                        32 => current_color = if style.bold {
                            egui::Color32::from_rgb(76, 230, 76)    // Bright Green #4ce64c
                        } else {
                            egui::Color32::from_rgb(68, 170, 68)    // Green #44aa44
                        },
                        33 => current_color = if style.bold {
                            if is_light_theme {
                                egui::Color32::from_rgb(167, 163, 33)  // Bright Yellow (light)
                            } else {
                                egui::Color32::from_rgb(222, 216, 44)  // Bright Yellow #ded82c
                            }
                        } else if is_light_theme {
                            egui::Color32::from_rgb(128, 64, 0)  // Darker orange for light theme
                        } else {
                            egui::Color32::from_rgb(170, 85, 0)  // Yellow #aa5500
                        },
                        34 => current_color = if style.bold {
                            egui::Color32::from_rgb(41, 95, 204)    // Bright Blue #295fcc
                        } else if is_light_theme {
                            egui::Color32::from_rgb(0, 43, 128)     // Darker blue for light theme
                        } else {
                            egui::Color32::from_rgb(0, 57, 170)     // Blue #0039aa
                        },
                        35 => current_color = if style.bold {
                            egui::Color32::from_rgb(204, 88, 204)   // Bright Magenta #cc58cc
                        } else {
                            egui::Color32::from_rgb(170, 34, 170)   // Magenta #aa22aa
                        },
                        36 => current_color = if style.bold {
                            egui::Color32::from_rgb(76, 204, 230)   // Bright Cyan #4ccce6
                        } else {
                            egui::Color32::from_rgb(26, 146, 170)   // Cyan #1a92aa
                        },
                        37 => current_color = if style.bold {
                            if is_light_theme {
                                egui::Color32::from_rgb(40, 40, 40)     // Bright White (light)
                            } else {
                                egui::Color32::from_rgb(255, 255, 255)  // Bright White #ffffff
                            }
                        } else if is_light_theme {
                            egui::Color32::from_rgb(80, 80, 80)  // Dark gray for light theme
                        } else {
                            egui::Color32::from_rgb(170, 170, 170)  // White #aaaaaa
                        },
                        39 => current_color = default_color,
                        // Bright/high-intensity foreground colors (90-97) - Xubuntu Dark palette
                        90 => current_color = egui::Color32::from_rgb(119, 119, 119), // Bright Black #777777
                        91 => current_color = egui::Color32::from_rgb(255, 135, 135), // Bright Red #ff8787
                        92 => current_color = egui::Color32::from_rgb(76, 230, 76),   // Bright Green #4ce64c
                        93 => current_color = if is_light_theme {
                            egui::Color32::from_rgb(167, 163, 33)  // Darker lime for light theme
                        } else {
                            egui::Color32::from_rgb(222, 216, 44)  // Bright Yellow #ded82c
                        },
                        94 => current_color = egui::Color32::from_rgb(41, 95, 204),   // Bright Blue #295fcc
                        95 => current_color = egui::Color32::from_rgb(204, 88, 204),  // Bright Magenta #cc58cc
                        96 => current_color = egui::Color32::from_rgb(76, 204, 230),  // Bright Cyan #4ccce6
                        97 => current_color = if is_light_theme {
                            egui::Color32::from_rgb(40, 40, 40)  // Near black for light theme
                        } else {
                            egui::Color32::from_rgb(255, 255, 255)  // Bright White #ffffff
                        },
                        // Extended foreground color modes
                        38 => {
                            // 38;5;n = 256-color, 38;2;r;g;b = 24-bit RGB
                            if i + 1 < parts.len() {
                                match parts[i + 1].parse::<u8>().unwrap_or(0) {
                                    5 => {
                                        // 256-color mode: 38;5;n
                                        if i + 2 < parts.len() {
                                            if let Ok(n) = parts[i + 2].parse::<u8>() {
                                                let (r, g, b) = Self::color256_to_rgb(n, is_light_theme);
                                                current_color = egui::Color32::from_rgb(r, g, b);
                                            }
                                            i += 2;
                                        }
                                    }
                                    2 => {
                                        // 24-bit RGB mode: 38;2;r;g;b
                                        if i + 4 < parts.len() {
                                            let r = parts[i + 2].parse::<u8>().unwrap_or(0);
                                            let g = parts[i + 3].parse::<u8>().unwrap_or(0);
                                            let b = parts[i + 4].parse::<u8>().unwrap_or(0);
                                            current_color = egui::Color32::from_rgb(r, g, b);
                                            i += 4;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Standard background colors (40-47) - Xubuntu Dark palette
                        40 => current_bg = egui::Color32::from_rgb(0, 0, 0),       // Black #000000
                        41 => current_bg = egui::Color32::from_rgb(170, 0, 0),     // Red #aa0000
                        42 => current_bg = egui::Color32::from_rgb(68, 170, 68),   // Green #44aa44
                        43 => current_bg = egui::Color32::from_rgb(170, 85, 0),    // Yellow #aa5500
                        44 => current_bg = egui::Color32::from_rgb(0, 57, 170),    // Blue #0039aa
                        45 => current_bg = egui::Color32::from_rgb(170, 34, 170),  // Magenta #aa22aa
                        46 => current_bg = egui::Color32::from_rgb(26, 146, 170),  // Cyan #1a92aa
                        47 => current_bg = egui::Color32::from_rgb(170, 170, 170), // White #aaaaaa
                        49 => current_bg = egui::Color32::TRANSPARENT,             // Default background
                        // Extended background color modes
                        48 => {
                            // 48;5;n = 256-color, 48;2;r;g;b = 24-bit RGB
                            if i + 1 < parts.len() {
                                match parts[i + 1].parse::<u8>().unwrap_or(0) {
                                    5 => {
                                        // 256-color mode: 48;5;n
                                        if i + 2 < parts.len() {
                                            if let Ok(n) = parts[i + 2].parse::<u8>() {
                                                let (r, g, b) = Self::color256_to_rgb(n, is_light_theme);
                                                current_bg = egui::Color32::from_rgb(r, g, b);
                                            }
                                            i += 2;
                                        }
                                    }
                                    2 => {
                                        // 24-bit RGB mode: 48;2;r;g;b
                                        if i + 4 < parts.len() {
                                            let r = parts[i + 2].parse::<u8>().unwrap_or(0);
                                            let g = parts[i + 3].parse::<u8>().unwrap_or(0);
                                            let b = parts[i + 4].parse::<u8>().unwrap_or(0);
                                            current_bg = egui::Color32::from_rgb(r, g, b);
                                            i += 4;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Bright/high-intensity background colors (100-107) - Xubuntu Dark palette
                        100 => current_bg = egui::Color32::from_rgb(119, 119, 119), // Bright Black #777777
                        101 => current_bg = egui::Color32::from_rgb(255, 135, 135), // Bright Red #ff8787
                        102 => current_bg = egui::Color32::from_rgb(76, 230, 76),   // Bright Green #4ce64c
                        103 => current_bg = egui::Color32::from_rgb(222, 216, 44),  // Bright Yellow #ded82c
                        104 => current_bg = egui::Color32::from_rgb(41, 95, 204),   // Bright Blue #295fcc
                        105 => current_bg = egui::Color32::from_rgb(204, 88, 204),  // Bright Magenta #cc58cc
                        106 => current_bg = egui::Color32::from_rgb(76, 204, 230),  // Bright Cyan #4ccce6
                        107 => current_bg = egui::Color32::from_rgb(255, 255, 255), // Bright White #ffffff
                        _ => {}
                    }
                    i += 1;
                }
            } else {
                // Check for colored square emoji and render as colored blocks
                if let Some((r, g, b)) = Self::colored_square_rgb(c) {
                    // Flush current segment first
                    if !segment.is_empty() {
                        Self::append_segment_with_shades(&segment, &font_id, current_color, current_bg, theme_bg, color_offset_percent, &style, job);
                        segment.clear();
                    }
                    // Add two block characters with the emoji's color
                    let square_color = egui::Color32::from_rgb(r, g, b);
                    job.append(
                        "██",
                        0.0,
                        egui::TextFormat {
                            font_id: font_id.clone(),
                            color: square_color,
                            background: current_bg,
                            ..Default::default()
                        },
                    );
                } else {
                    segment.push(c);
                }
            }
        }

        // Flush remaining segment
        if !segment.is_empty() {
            Self::append_segment_with_shades(&segment, &font_id, current_color, current_bg, theme_bg, color_offset_percent, &style, job);
        }
    }

    /// Get the RGB color for a colored square emoji
    fn colored_square_rgb(c: char) -> Option<(u8, u8, u8)> {
        match c {
            '🟥' => Some((0xDD, 0x2E, 0x44)),
            '🟧' => Some((0xF4, 0x90, 0x0C)),
            '🟨' => Some((0xFD, 0xCB, 0x58)),
            '🟩' => Some((0x78, 0xB1, 0x59)),
            '🟦' => Some((0x55, 0xAC, 0xEE)),
            '🟪' => Some((0xAA, 0x8E, 0xD6)),
            '🟫' => Some((0xA0, 0x6A, 0x42)),
            '⬛' => Some((0x31, 0x37, 0x3D)),
            '⬜' => Some((0xE6, 0xE7, 0xE8)),
            _ => None,
        }
    }

    /// Find URLs in text and return their character positions (not byte positions)
    /// Returns (start_char_idx, end_char_idx, url_string)
    /// O(n) algorithm: searches directly on &str byte offsets, no per-iteration allocations
    fn find_urls(text: &str) -> Vec<(usize, usize, String)> {
        let mut urls = Vec::new();
        let mut search_start = 0usize; // byte offset

        while search_start < text.len() {
            let remaining = &text[search_start..];
            let byte_pos = match (remaining.find("http://"), remaining.find("https://")) {
                (Some(h), Some(hs)) => Some(h.min(hs)),
                (Some(h), None) => Some(h),
                (None, Some(hs)) => Some(hs),
                (None, None) => None,
            };

            if let Some(offset) = byte_pos {
                let url_start_byte = search_start + offset;
                let mut url_end_byte = url_start_byte;
                for (i, c) in text[url_start_byte..].char_indices() {
                    if c.is_whitespace() || matches!(c, '>' | '"' | '\'' | ')' | ']') {
                        url_end_byte = url_start_byte + i;
                        break;
                    }
                    url_end_byte = url_start_byte + i + c.len_utf8();
                }

                if url_end_byte > url_start_byte {
                    // Convert byte offsets to char indices
                    let start_char = text[..url_start_byte].chars().count();
                    let url_str = &text[url_start_byte..url_end_byte];
                    let end_char = start_char + url_str.chars().count();
                    urls.push((start_char, end_char, url_str.to_string()));
                }
                search_start = url_end_byte;
            } else {
                break;
            }
        }
        urls
    }

    /// Open a URL in the default browser
    fn open_url(url: &str) {
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("xdg-open")
                .arg(url)
                .spawn();
        }
        #[cfg(target_os = "android")]
        {
            let _ = std::process::Command::new("xdg-open")
                .arg(url)
                .spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg(url)
                .spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", url])
                .spawn();
        }
    }

    /// Get color for a colored square emoji, if it is one
    fn colored_square_color(c: char) -> Option<egui::Color32> {
        match c {
            '🟥' => Some(egui::Color32::from_rgb(0xDD, 0x2E, 0x44)), // Red
            '🟧' => Some(egui::Color32::from_rgb(0xF4, 0x90, 0x0C)), // Orange
            '🟨' => Some(egui::Color32::from_rgb(0xFD, 0xCB, 0x58)), // Yellow
            '🟩' => Some(egui::Color32::from_rgb(0x78, 0xB1, 0x59)), // Green
            '🟦' => Some(egui::Color32::from_rgb(0x55, 0xAC, 0xEE)), // Blue
            '🟪' => Some(egui::Color32::from_rgb(0xAA, 0x8E, 0xD6)), // Purple
            '🟫' => Some(egui::Color32::from_rgb(0xA0, 0x6A, 0x42)), // Brown
            '⬛' => Some(egui::Color32::from_rgb(0x31, 0x37, 0x3D)), // Black
            '⬜' => Some(egui::Color32::from_rgb(0xE6, 0xE7, 0xE8)), // White
            _ => None,
        }
    }

    /// Parse text into segments of plain text, Discord emojis, and colored squares
    fn parse_discord_segments(text: &str) -> Vec<DiscordSegment> {
        use regex::Regex;

        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"<(a?):([^:]+):(\d+)>").unwrap()
        });

        // First pass: handle Discord custom emoji
        let mut temp_segments: Vec<DiscordSegment> = Vec::new();
        let mut last_end = 0;

        for cap in re.captures_iter(text) {
            let m = cap.get(0).unwrap();
            if m.start() > last_end {
                temp_segments.push(DiscordSegment::Text(text[last_end..m.start()].to_string()));
            }
            let animated = &cap[1] == "a";
            let name = cap[2].to_string();
            let id = cap[3].to_string();
            temp_segments.push(DiscordSegment::Emoji { name, id, animated });
            last_end = m.end();
        }

        if last_end < text.len() {
            temp_segments.push(DiscordSegment::Text(text[last_end..].to_string()));
        }

        if temp_segments.is_empty() {
            temp_segments.push(DiscordSegment::Text(text.to_string()));
        }

        // Second pass: split Text segments to extract colored square emoji
        let mut segments: Vec<DiscordSegment> = Vec::new();
        for seg in temp_segments {
            match seg {
                DiscordSegment::Text(txt) => {
                    // Split text at colored square emoji
                    let mut current_text = String::new();
                    for c in txt.chars() {
                        if let Some(color) = Self::colored_square_color(c) {
                            if !current_text.is_empty() {
                                segments.push(DiscordSegment::Text(current_text.clone()));
                                current_text.clear();
                            }
                            segments.push(DiscordSegment::ColoredSquare(color));
                        } else {
                            current_text.push(c);
                        }
                    }
                    if !current_text.is_empty() {
                        segments.push(DiscordSegment::Text(current_text));
                    }
                }
                other => segments.push(other),
            }
        }

        if segments.is_empty() {
            segments.push(DiscordSegment::Text(String::new()));
        }

        segments
    }

    /// Check if text contains Discord custom emojis (not colored squares)
    /// Colored squares are now handled in the non-emoji path for better selection support
    fn has_discord_emojis(text: &str) -> bool {
        // Only use emoji rendering path for actual Discord custom emojis
        // Colored squares are rendered as colored blocks in the LayoutJob path
        text.contains("<:") || text.contains("<a:")
    }

    /// Insert zero-width spaces after break characters in long words (>15 chars)
    /// Break characters: [ ] ( ) , \ / - & = ? and spaces
    /// Note: '.' is excluded because it breaks filenames (image.png) and domains awkwardly
    /// Skips ANSI escape sequences to avoid corrupting them
    fn insert_word_breaks(text: &str) -> String {
        const ZWSP: char = '\u{200B}'; // Zero-width space
        const BREAK_CHARS: &[char] = &['[', ']', '(', ')', ',', '\\', '/', '-', '&', '=', '?', '.', ';', ' '];
        const MIN_WORD_LEN: usize = 15;

        let mut result = String::with_capacity(text.len() * 2);
        let mut word_len = 0;
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            result.push(c);

            // Skip ANSI escape sequences entirely
            if c == '\x1b' && chars.peek() == Some(&'[') {
                // Consume the '['
                if let Some(bracket) = chars.next() {
                    result.push(bracket);
                }
                // Consume until we hit the terminator (alphabetic or ~)
                while let Some(&sc) = chars.peek() {
                    result.push(chars.next().unwrap());
                    if sc.is_ascii_alphabetic() || sc == '~' {
                        break;
                    }
                }
                continue;
            }

            if c.is_whitespace() {
                word_len = 0;
            } else {
                word_len += 1;
                // Insert break opportunity after break chars in long words
                if word_len > MIN_WORD_LEN && BREAK_CHARS.contains(&c) {
                    result.push(ZWSP);
                }
            }
        }

        result
    }

    /// Render a line with inline Discord emoji images and clickable URLs
    fn render_line_with_emojis(
        ui: &mut egui::Ui,
        text: &str,
        default_color: egui::Color32,
        font_id: &egui::FontId,
        is_light_theme: bool,
        link_color: egui::Color32,
        color_offset_percent: u8,
        blink_visible: bool,
    ) {
        let segments = Self::parse_discord_segments(text);
        let available_width = ui.available_width();

        // Check if this line has Discord emoji or colored squares
        let has_special = segments.iter().any(|s| matches!(s, DiscordSegment::Emoji { .. } | DiscordSegment::ColoredSquare(_)));

        if has_special {
            // Use horizontal_wrapped for lines with emoji/colored squares
            // URL clicking won't work here, but emoji will render correctly
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for segment in segments {
                    match segment {
                        DiscordSegment::Text(txt) => {
                            let txt_with_breaks = Self::insert_word_breaks(&txt);
                            let mut job = egui::text::LayoutJob {
                                wrap: egui::text::TextWrapping {
                                    max_width: available_width,
                                    ..Default::default()
                                },
                                ..Default::default()
                            };
                            Self::append_ansi_to_job(&txt_with_breaks, default_color, font_id.clone(), &mut job, is_light_theme, color_offset_percent, blink_visible);
                            let galley = ui.fonts(|f| f.layout_job(job));
                            ui.label(galley);
                        }
                        DiscordSegment::Emoji { name, id, animated } => {
                            let ext = if animated { "gif" } else { "png" };
                            let url = format!("https://cdn.discordapp.com/emojis/{}.{}", id, ext);
                            let image = egui::Image::from_uri(&url)
                                .fit_to_exact_size(egui::vec2(font_id.size * 1.2, font_id.size * 1.2));
                            ui.add(image).on_hover_text(format!(":{}:", name));
                        }
                        DiscordSegment::ColoredSquare(color) => {
                            let size = font_id.size * 1.1;
                            let (rect, _response) = ui.allocate_exact_size(
                                egui::vec2(size, size),
                                egui::Sense::hover()
                            );
                            if ui.is_rect_visible(rect) {
                                ui.painter().rect_filled(rect, 2.0, color);
                            }
                        }
                    }
                }
            });
        } else {
            // No emoji/colored squares - use LayoutJob with clickable URLs
            let mut job = egui::text::LayoutJob {
                wrap: egui::text::TextWrapping {
                    max_width: available_width,
                    ..Default::default()
                },
                ..Default::default()
            };

            // Track URL positions in the final job text (character indices)
            let mut url_ranges: Vec<(usize, usize, String)> = Vec::new();

            for segment in &segments {
                if let DiscordSegment::Text(txt) = segment {
                    let urls = Self::find_urls(txt);
                    if urls.is_empty() {
                        let txt_with_breaks = Self::insert_word_breaks(txt);
                        Self::append_ansi_to_job(&txt_with_breaks, default_color, font_id.clone(), &mut job, is_light_theme, color_offset_percent, blink_visible);
                    } else {
                        let char_to_byte: Vec<usize> = txt.char_indices().map(|(i, _)| i).collect();
                        let txt_char_count = char_to_byte.len();
                        let mut last_end_char = 0;

                        for (start_char, end_char, url) in urls {
                            if start_char > last_end_char {
                                let start_byte = char_to_byte.get(last_end_char).copied().unwrap_or(0);
                                let end_byte = char_to_byte.get(start_char).copied().unwrap_or(txt.len());
                                let before = Self::insert_word_breaks(&txt[start_byte..end_byte]);
                                Self::append_ansi_to_job(&before, default_color, font_id.clone(), &mut job, is_light_theme, color_offset_percent, blink_visible);
                            }

                            let clean_url = crate::util::strip_ansi_codes(&url);
                            let url_with_breaks = Self::insert_word_breaks(&clean_url);
                            let url_start = job.text.chars().count();
                            job.append(&url_with_breaks, 0.0, egui::TextFormat {
                                font_id: font_id.clone(),
                                color: link_color,
                                underline: egui::Stroke::new(1.0, link_color),
                                ..Default::default()
                            });
                            let url_end = job.text.chars().count();
                            let final_url = clean_url.replace('\u{200B}', "");
                            url_ranges.push((url_start, url_end, final_url));

                            last_end_char = end_char;
                        }

                        if last_end_char < txt_char_count {
                            let start_byte = char_to_byte.get(last_end_char).copied().unwrap_or(txt.len());
                            let after = Self::insert_word_breaks(&txt[start_byte..]);
                            Self::append_ansi_to_job(&after, default_color, font_id.clone(), &mut job, is_light_theme, color_offset_percent, blink_visible);
                        }
                    }
                }
            }

            let galley = ui.fonts(|f| f.layout_job(job));
            // Allocate space with click_and_drag to allow text selection
            let (rect, _response) = ui.allocate_exact_size(galley.size(), egui::Sense::click_and_drag());
            let text_pos = rect.min;

            // Paint the galley
            ui.painter().galley(text_pos, galley.clone());

            // Check for clicks and hovers using global input state
            let pointer_pos = ui.input(|i| i.pointer.interact_pos());
            let hover_pos = ui.input(|i| i.pointer.hover_pos());
            let primary_clicked = ui.input(|i| i.pointer.primary_clicked());

            // Handle URL clicks - check if click is within our rect
            if primary_clicked && !url_ranges.is_empty() {
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        let relative_pos = pos - text_pos;
                        let cursor = galley.cursor_from_pos(relative_pos);
                        let click_char = cursor.ccursor.index;

                        for (start, end, url) in &url_ranges {
                            if click_char >= *start && click_char < *end {
                                // Strip zero-width spaces that were inserted for word breaking
                                let clean_url = url.replace('\u{200B}', "");
                                Self::open_url(&clean_url);
                                break;
                            }
                        }
                    }
                }
            }

            // Show pointer cursor when hovering over URLs
            if let Some(pos) = hover_pos {
                if rect.contains(pos) {
                    let relative_pos = pos - text_pos;
                    let cursor = galley.cursor_from_pos(relative_pos);
                    let hover_char = cursor.ccursor.index;

                    for (start, end, _) in &url_ranges {
                        if hover_char >= *start && hover_char < *end {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for RemoteGuiApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Use transparent clear color so window transparency works
        let theme = self.theme.clone();
        let bg = theme.bg();
        let a = self.transparency;
        [bg.r() as f32 / 255.0, bg.g() as f32 / 255.0, bg.b() as f32 / 255.0, a]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Skip rendering until screen has valid dimensions
        // This prevents NaN panics from egui layout calculations on startup
        let screen = ctx.screen_rect();
        if screen.width() <= 0.0 || screen.height() <= 0.0 ||
           screen.width().is_nan() || screen.height().is_nan() ||
           !screen.width().is_finite() || !screen.height().is_finite() {
            ctx.request_repaint();
            return;
        }

        // Process incoming WebSocket messages
        let had_messages = self.process_messages();
        if had_messages {
            ctx.request_repaint(); // Immediate repaint when new data arrives
        }

        // Handle reload reconnect with retry
        if let Some(reconnect_at) = self.reload_reconnect_at {
            if std::time::Instant::now() >= reconnect_at {
                self.reload_reconnect_attempts += 1;
                if self.reload_reconnect_attempts <= 5 {
                    self.connect_websocket();
                    self.reload_reconnect_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(1));
                } else {
                    self.reload_reconnect_at = None;
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Blink animation: toggle every 500ms (only when blink text exists)
        if self.has_blink_text {
            if self.blink_last_toggle.elapsed() >= std::time::Duration::from_millis(500) {
                self.blink_visible = !self.blink_visible;
                self.blink_last_toggle = std::time::Instant::now();
                self.output_dirty = true;
                ctx.request_repaint();
            }
            let time_since_toggle = self.blink_last_toggle.elapsed();
            let next_toggle = std::time::Duration::from_millis(500).saturating_sub(time_since_toggle);
            ctx.request_repaint_after(next_toggle);
        }

        // Apply theme to egui visuals (clone to avoid borrowing self)
        let theme = self.theme.clone();
        let mut visuals = if theme.is_dark() {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        // Customize based on our theme
        // NOTE: Do NOT set override_text_color as it overrides LayoutJob colors!
        visuals.override_text_color = None;
        // Apply transparency to panel and window fills
        let alpha = (self.transparency * 255.0) as u8;
        let panel_bg = theme.panel_bg();
        visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(panel_bg.r(), panel_bg.g(), panel_bg.b(), alpha);
        visuals.window_fill = egui::Color32::from_rgba_unmultiplied(panel_bg.r(), panel_bg.g(), panel_bg.b(), alpha);
        visuals.widgets.noninteractive.bg_fill = theme.button_bg();
        visuals.widgets.inactive.bg_fill = theme.button_bg();
        visuals.widgets.hovered.bg_fill = theme.selection_bg();
        visuals.widgets.active.bg_fill = theme.selection_bg();
        visuals.selection.bg_fill = theme.selection_bg();
        visuals.extreme_bg_color = theme.bg();
        visuals.faint_bg_color = theme.bg();
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
        // Set proper foreground strokes for buttons/widgets to be visible
        let fg_stroke = egui::Stroke::new(1.0, theme.fg());
        visuals.widgets.noninteractive.fg_stroke = fg_stroke;
        visuals.widgets.inactive.fg_stroke = fg_stroke;
        visuals.widgets.hovered.fg_stroke = fg_stroke;
        visuals.widgets.active.fg_stroke = fg_stroke;
        ctx.set_visuals(visuals);

        // Update Windows title bar color to match theme
        #[cfg(target_os = "windows")]
        {
            let need_update = match &self.titlebar_theme {
                None => true,
                Some(prev) => prev.colors != theme.colors,
            };
            if need_update {
                if windows_titlebar::apply_titlebar_theme(theme.is_dark()) {
                    self.titlebar_theme = Some(theme.clone());
                }
            }
        }

        // Make scrollbars always visible and solid (not floating)
        let mut style = (*ctx.style()).clone();
        style.spacing.scroll = egui::style::ScrollStyle {
            floating: false,  // Solid scrollbar, always takes space
            bar_width: 10.0,
            handle_min_length: 20.0,
            bar_inner_margin: 2.0,
            bar_outer_margin: 2.0,
            ..Default::default()
        };
        // Reduce spacing for tighter text layout (helps with ASCII art)
        style.spacing.item_spacing.y = 0.0;
        ctx.set_style(style);

        // Load custom font if font name or tweaks changed
        let tweaks_changed = self.loaded_font_scale != self.font_scale
            || self.loaded_font_y_offset != self.font_y_offset
            || self.loaded_font_baseline_offset != self.font_baseline_offset;
        if self.loaded_font_name != self.font_name || tweaks_changed {
            self.loaded_font_name = self.font_name.clone();
            self.loaded_font_scale = self.font_scale;
            self.loaded_font_y_offset = self.font_y_offset;
            self.loaded_font_baseline_offset = self.font_baseline_offset;

            let mut fonts = egui::FontDefinitions::default();

            if !self.font_name.is_empty() {
                // Try to load the system font
                if let Some(font_data) = Self::find_system_font(&self.font_name) {
                    // Add the custom font with user-configurable tweaks
                    let font_data = egui::FontData::from_owned(font_data).tweak(
                        egui::FontTweak {
                            scale: self.font_scale,
                            y_offset_factor: self.font_y_offset,
                            y_offset: 0.0,
                            baseline_offset_factor: self.font_baseline_offset,
                        }
                    );
                    fonts.font_data.insert(
                        "custom_mono".to_owned(),
                        font_data,
                    );

                    // Make it the first priority for monospace
                    fonts.families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, "custom_mono".to_owned());

                    // Also use for proportional text
                    fonts.families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .insert(0, "custom_mono".to_owned());
                }
            }
            // Load FreeMono before NotoEmoji-Regular in the fallback chain.
            // FreeMono has a proper sparkles glyph for ✨; NotoEmoji-Regular renders it as a diamond.
            // Default monospace chain is [Hack, Ubuntu-Light, NotoEmoji-Regular, emoji-icon-font].
            // We insert FreeMono before NotoEmoji-Regular so it gets priority for ✨.
            if let Ok(font_data) = std::fs::read("/usr/share/fonts/truetype/freefont/FreeMono.ttf") {
                fonts.font_data.insert("freemono".to_owned(), egui::FontData::from_owned(font_data));
                // Insert before NotoEmoji-Regular in monospace chain
                let mono_family = fonts.families.entry(egui::FontFamily::Monospace).or_default();
                if let Some(pos) = mono_family.iter().position(|n| n == "NotoEmoji-Regular") {
                    mono_family.insert(pos, "freemono".to_owned());
                } else {
                    mono_family.push("freemono".to_owned());
                }
                fonts.families.entry(egui::FontFamily::Proportional).or_default().push("freemono".to_owned());
            }

            // Load additional symbol fonts for Unicode/emoji coverage
            let symbol_fonts: &[(&str, &str)] = &[
                ("symbols2", "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf"),
                ("symbols", "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf"),
                ("symbola", "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf"),
            ];
            for (name, path) in symbol_fonts {
                if let Ok(font_data) = std::fs::read(path) {
                    fonts.font_data.insert(
                        (*name).to_owned(),
                        egui::FontData::from_owned(font_data),
                    );
                    fonts.families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .push((*name).to_owned());
                    fonts.families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push((*name).to_owned());
                }
            }

            ctx.set_fonts(fonts);
        }

        // Apply font size to monospace text style
        let mut style = (*ctx.style()).clone();
        if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Monospace) {
            font_id.size = self.font_size;
        }
        // Also apply to body text
        if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
            font_id.size = self.font_size;
        }
        ctx.set_style(style);

        // Request repaint after a short delay to keep polling messages
        // Using request_repaint_after instead of request_repaint allows egui to
        // prioritize immediate keyboard/mouse events over background polling
        ctx.request_repaint_after(std::time::Duration::from_millis(50));

        // Wait for settings before rendering UI (prevents theme flash)
        // Visuals are already applied above so the background color is correct
        if self.authenticated && !self.settings_received {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(theme.bg()))
                .show(ctx, |ui| {
                    // Show waiting indicator after a brief delay
                    if self.connect_time.is_some_and(|t| t.elapsed().as_millis() > 500)
                        || (self.is_master && self.frame_count > 30)
                    {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            ui.label(egui::RichText::new("Waiting for server state...")
                                .color(theme.fg_muted())
                                .size(14.0));
                        });
                    }
                });
            self.frame_count += 1;
            ctx.request_repaint();
            return;
        }

        if !self.connected || !self.authenticated {
            // Show login dialog with dog and Clay branding
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(theme.bg_deep()))
                .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);

                    // Clay logo image (clay2.png scaled to ~200px wide, preserving aspect ratio)
                    let splash_image = egui::Image::from_bytes(
                        "bytes://clay_splash",
                        include_bytes!("../clay2.png"),
                    ).fit_to_exact_size(egui::vec2(200.0, 200.0));
                    ui.add(splash_image);

                    let tagline_color = egui::Color32::from_rgb(0xff, 0x87, 0xff);  // 213
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new("A 90dies mud client written today").color(tagline_color).italics());
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new("/help for how to use clay").color(theme.fg_muted()));
                    ui.add_space(20.0);

                    // Auto-connect on first frame to check if allow list grants access
                    if !self.auto_connect_attempted && !self.connected {
                        self.auto_connect_attempted = true;
                        self.connect_websocket();
                    }

                    // Check if we're still waiting for allow list response (500ms timeout)
                    let allow_list_timeout = std::time::Duration::from_millis(500);
                    let still_checking_allow_list = self.connected
                        && !self.authenticated
                        && !self.password_submitted
                        && self.connect_time.is_some_and(|t| t.elapsed() < allow_list_timeout);

                    // Login card with Frame
                    egui::Frame::none()
                        .fill(theme.bg_surface())
                        .stroke(egui::Stroke::new(1.0, theme.border_subtle()))
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::same(20.0))
                        .show(ui, |ui| {
                            // Server address
                            ui.label(egui::RichText::new(format!("Connecting to {}", self.ws_url))
                                .color(theme.fg_muted())
                                .size(11.0));
                            ui.add_space(16.0);

                            // Show connection status or password prompt
                            if still_checking_allow_list {
                                ui.label(egui::RichText::new("Checking allow list...")
                                    .color(theme.fg_secondary()));
                                ui.add_space(10.0);
                                // Request repaint to update when timeout expires
                                ctx.request_repaint();
                            }

                            // Username field (only in multiuser mode)
                            if self.multiuser_mode {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(280.0, 14.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.label(egui::RichText::new("USERNAME")
                                            .color(theme.fg_muted())
                                            .size(10.0));
                                    }
                                );
                                ui.add_space(4.0);

                                let username_edit = TextEdit::singleline(&mut self.username)
                                    .desired_width(280.0)
                                    .margin(egui::vec2(12.0, 8.0));
                                ui.add(username_edit);
                                ui.add_space(12.0);
                            }

                            // Password label - left justified with field
                            ui.allocate_ui_with_layout(
                                egui::vec2(280.0, 14.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new("PASSWORD")
                                        .color(theme.fg_muted())
                                        .size(10.0));
                                }
                            );
                            ui.add_space(4.0);

                            // Password input with custom styling
                            let password_edit = TextEdit::singleline(&mut self.password)
                                .password(true)
                                .desired_width(280.0)
                                .margin(egui::vec2(12.0, 8.0));
                            let response = ui.add(password_edit);

                            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                self.password_submitted = true;
                                if self.connected {
                                    self.send_auth();
                                } else {
                                    self.connect_websocket();
                                }
                            }

                            ui.add_space(15.0);

                            // Connect button - styled as primary, right justified with password field
                            let button_size = egui::vec2(80.0, 32.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(280.0, button_size.y),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let button = egui::Button::new(
                                        egui::RichText::new("CONNECT")
                                            .color(theme.bg_deep())
                                            .strong()
                                            .size(11.0)
                                    )
                                    .fill(theme.accent_dim())
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding::same(4.0));

                                    if ui.add_sized(button_size, button).clicked() {
                                        self.password_submitted = true;
                                        if self.connected {
                                            self.send_auth();
                                        } else {
                                            self.connect_websocket();
                                        }
                                    }
                                }
                            );

                            // Error message
                            if let Some(ref err) = self.error_message {
                                ui.add_space(12.0);
                                ui.label(egui::RichText::new(err.as_str())
                                    .color(theme.error())
                                    .size(12.0));
                            }
                        });
                });
            });
        } else {
            // Show main interface with menu bar
            let mut action: Option<&str> = None;
            let mut cursor_home = false;

            // Handle keyboard shortcuts (only when no popup is open)
            if self.popup_state == PopupState::None && !self.filter_active {
                let switch_world: Option<usize> = None;
                let mut history_action: Option<i32> = None; // -1 = prev, 1 = next
                let mut scroll_action: Option<i32> = None; // -1 = up, 1 = down
                let mut copy_selection = false;
                let mut clear_input = false;
                let mut delete_word = false;
                let mut resize_input: i32 = 0;
                let mut tab_complete = false;

                // Use input_mut to consume events before widgets get them
                ctx.input_mut(|i| {
                    // Ctrl+key shortcuts
                    if i.modifiers.ctrl {
                        if i.consume_key(egui::Modifiers::CTRL, egui::Key::L) {
                            action = Some("redraw");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::E) {
                            action = Some("edit_current");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::S) {
                            action = Some("setup");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::U) {
                            // Ctrl+U - clear input (like console)
                            clear_input = true;
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::O) {
                            action = Some("connect");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::D) {
                            action = Some("disconnect");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::P) {
                            // Ctrl+P - previous command history
                            history_action = Some(-1);
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::N) {
                            // Ctrl+N - next command history
                            history_action = Some(1);
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::W) {
                            // Ctrl+W - delete word before cursor
                            delete_word = true;
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Q) {
                            // Ctrl+Q - spell check
                            action = Some("spell_check");
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::A) {
                            // Ctrl+A - move cursor to beginning of line
                            cursor_home = true;
                        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::C) {
                            // Ctrl+C - copy selected text
                            copy_selection = true;
                        }
                    } else if i.modifiers.alt {
                        // Alt+Up/Down - resize input area
                        if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp) {
                            resize_input = -1;
                        } else if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown) {
                            resize_input = 1;
                        } else if i.consume_key(egui::Modifiers::ALT, egui::Key::J) {
                            // Alt+J (Escape+j) - Jump to end, release all pending
                            if self.current_world < self.worlds.len()
                                && self.worlds[self.current_world].pending_count > 0
                            {
                                if let Some(ref tx) = self.ws_tx {
                                    let _ = tx.send(WsMessage::ReleasePending {
                                        world_index: self.current_world,
                                        count: 0, // 0 = release all
                                    });
                                }
                            }
                            self.scroll_offset = None; // Scroll to bottom
                            self.scroll_jump_to = Some(self.scroll_max_offset);
                        }
                        // Note: Ctrl+Up/Down are handled by first ctrl block, letting egui TextEdit
                        // handle cursor movement in multi-line input if not consumed
                    } else {
                        // Non-modified keys - consume to prevent widgets from handling
                        if i.consume_key(egui::Modifiers::NONE, egui::Key::PageUp) {
                            scroll_action = Some(-1);
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::PageDown) {
                            scroll_action = Some(1);
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F2) {
                            // F2 - toggle MUD tag display
                            self.show_tags = !self.show_tags;
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F4) {
                            // F4 - toggle filter popup
                            self.filter_active = true;
                            self.filter_text.clear();
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F8) {
                            // F8 - toggle action pattern highlighting
                            self.highlight_actions = !self.highlight_actions;
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F9) {
                            // F9 - toggle GMCP user processing for current world
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::ToggleWorldGmcp { world_index: self.current_world });
                            }
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                            // Tab - command completion if input starts with / or #
                            // Otherwise release pending lines or scroll down if viewing history
                            if self.input_buffer.starts_with('/') {
                                tab_complete = true;
                            } else if self.current_world < self.worlds.len()
                                && self.worlds[self.current_world].pending_count > 0
                            {
                                // Send ReleasePending to server with this client's visible line count
                                // Server will release lines and broadcast PendingLinesUpdate to sync all clients
                                let release_count = self.output_visible_lines.saturating_sub(2).max(1);
                                if let Some(ref tx) = self.ws_tx {
                                    let _ = tx.send(WsMessage::ReleasePending {
                                        world_index: self.current_world,
                                        count: release_count,
                                    });
                                }
                                // Scroll to bottom
                                self.scroll_offset = None;
                                self.scroll_jump_to = Some(self.scroll_max_offset);
                            } else if self.scroll_offset.is_some() {
                                // Viewing history - scroll down like PgDn
                                if let Some(offset) = self.scroll_offset {
                                    let new_offset = offset + 300.0;
                                    if new_offset >= self.scroll_max_offset - 10.0 {
                                        self.scroll_offset = None; // Snap to bottom
                                        self.scroll_jump_to = Some(self.scroll_max_offset);
                                    } else {
                                        self.scroll_offset = Some(new_offset);
                                        self.scroll_jump_to = Some(new_offset);
                                    }
                                }
                            }
                        }

                        // Up/Down arrow - request world switch from server
                        // Server calculates using centralized unseen tracking
                        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::CalculatePrevWorld { current_index: self.current_world });
                            }
                        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::CalculateNextWorld { current_index: self.current_world });
                            }
                        }
                    }

                    // Remove Up/Down events from the event list entirely so the multiline
                    // TextEdit can't process them for cursor movement (consume_key alone
                    // isn't sufficient since TextEdit iterates the raw event list directly)
                    i.events.retain(|e| !matches!(e,
                        egui::Event::Key { key: egui::Key::ArrowUp, pressed: true, modifiers, .. }
                        | egui::Event::Key { key: egui::Key::ArrowDown, pressed: true, modifiers, .. }
                        if !modifiers.ctrl && !modifiers.alt && !modifiers.shift
                    ));
                });

                // Apply clear input
                if clear_input {
                    self.input_buffer.clear();
                    self.history_index = 0;
                }

                // Apply copy selection (Ctrl+C)
                if copy_selection
                    && self.selection_start.is_some() && self.selection_end.is_some() {
                    let sel_id = egui::Id::new(format!("output_selection_{}", self.current_world));
                    let stored: Option<String> = ctx.data(|d| d.get_temp(sel_id));
                    if let Some(text) = stored {
                        ctx.copy_text(text);
                    }
                }

                // Apply delete word
                if delete_word && !self.input_buffer.is_empty() {
                    // Delete one word before cursor (end of buffer)
                    // First, skip any trailing whitespace
                    while self.input_buffer.ends_with(|c: char| c.is_whitespace()) {
                        self.input_buffer.pop();
                    }
                    // Then delete the word (non-whitespace characters)
                    while !self.input_buffer.is_empty()
                        && !self.input_buffer.ends_with(|c: char| c.is_whitespace())
                    {
                        self.input_buffer.pop();
                    }
                }

                // Apply tab completion
                let is_cmd_prefix = self.input_buffer.starts_with('/');
                if tab_complete && is_cmd_prefix {
                    // Get the partial command (everything up to first space)
                    let input = self.input_buffer.clone();
                    let (partial, args) = if let Some(space_pos) = input.find(' ') {
                        (&input[..space_pos], &input[space_pos..])
                    } else {
                        (input.as_str(), "")
                    };

                    let matches = {
                        // Unified / commands: Clay commands + TF commands + manual actions
                        let internal_commands = vec![
                            // Clay-specific commands
                            "/help", "/disconnect", "/dc", "/worlds", "/connections",
                            "/setup", "/web", "/actions", "/keepalive", "/reload", "/quit", "/gag",
                            "/testmusic", "/debug", "/dump", "/menu", "/notify", "/edit", "/tag",
                            // TF commands (now available with / prefix)
                            "/set", "/unset", "/let", "/echo", "/send", "/beep", "/quote",
                            "/expr", "/test", "/eval", "/if", "/elseif", "/else", "/endif",
                            "/while", "/done", "/for", "/break", "/def", "/undef", "/undefn",
                            "/undeft", "/list", "/purge", "/bind", "/unbind", "/load", "/save",
                            "/lcd", "/time", "/version", "/ps", "/kill", "/sh", "/recall",
                            "/setenv", "/listvar", "/repeat", "/fg", "/trigger", "/input",
                            "/grab", "/ungag", "/exit", "/connect", "/addworld",
                            // TF-specific versions (for conflicting commands)
                            "/tfhelp", "/tfgag",
                        ];

                        // Get manual actions (empty pattern)
                        let manual_actions: Vec<String> = self.actions.iter()
                            .filter(|a| a.pattern.is_empty())
                            .map(|a| format!("/{}", a.name))
                            .collect();

                        // Find all matches
                        let partial_lower = partial.to_lowercase();
                        let mut m: Vec<String> = internal_commands.iter()
                            .filter(|cmd| cmd.to_lowercase().starts_with(&partial_lower))
                            .map(|s| s.to_string())
                            .collect();
                        m.extend(manual_actions.iter()
                            .filter(|cmd| cmd.to_lowercase().starts_with(&partial_lower))
                            .cloned());
                        m.sort();
                        m.dedup();
                        m
                    };

                    if !matches.is_empty() {
                        // Find next match index
                        let next_idx = if partial.to_lowercase() == self.completion_prefix.to_lowercase() {
                            // Cycle to next match
                            (self.completion_index + 1) % matches.len()
                        } else {
                            // Find current match if we're already on a completed command
                            matches.iter()
                                .position(|m| m.eq_ignore_ascii_case(partial))
                                .map(|idx| (idx + 1) % matches.len())
                                .unwrap_or(0)
                        };

                        // Update completion state
                        self.completion_prefix = partial.to_string();
                        self.completion_index = next_idx;

                        // Replace input with completion
                        self.input_buffer = format!("{}{}", matches[next_idx], args);
                    }
                }

                // Apply input resize
                if resize_input != 0 {
                    let new_height = (self.input_height as i32 + resize_input).clamp(1, 15) as u16;
                    self.input_height = new_height;
                }

                // Apply history navigation
                if let Some(dir) = history_action {
                    if dir < 0 {
                        // Previous (older)
                        if self.history_index == 0 && !self.command_history.is_empty() {
                            self.saved_input = self.input_buffer.clone();
                            self.history_index = 1;
                            self.input_buffer = self.command_history[self.command_history.len() - 1].clone();
                        } else if self.history_index > 0 && self.history_index < self.command_history.len() {
                            self.history_index += 1;
                            let idx = self.command_history.len() - self.history_index;
                            self.input_buffer = self.command_history[idx].clone();
                        }
                    } else {
                        // Next (newer)
                        if self.history_index > 1 {
                            self.history_index -= 1;
                            let idx = self.command_history.len() - self.history_index;
                            self.input_buffer = self.command_history[idx].clone();
                        } else if self.history_index == 1 {
                            self.history_index = 0;
                            self.input_buffer = std::mem::take(&mut self.saved_input);
                        }
                    }
                }

                // Apply world switch (GUI-local only, doesn't affect console)
                if let Some(new_world) = switch_world {
                    if new_world != self.current_world && new_world < self.worlds.len() {
                        self.current_world = new_world;
                        self.selection_start = None; self.selection_end = None;
                        self.worlds[new_world].unseen_lines = 0;
                        self.scroll_offset = None; // Reset scroll
                        // Notify server and request current state
                        if let Some(ref tx) = self.ws_tx {
                            let _ = tx.send(WsMessage::MarkWorldSeen { world_index: new_world });
                            let _ = tx.send(WsMessage::RequestWorldState { world_index: new_world });
                        }
                    }
                }

                // Handle scroll action (will be used by scroll area)
                // In egui, scroll_offset is from the TOP, so:
                // - PageUp = decrease offset (scroll towards top/older content)
                // - PageDown = increase offset (scroll towards bottom/newer content)
                if let Some(dir) = scroll_action {
                    if dir < 0 {
                        // Scroll up (PageUp) - decrease offset to show older content
                        if let Some(offset) = self.scroll_offset {
                            let new_offset = (offset - 300.0).max(0.0);
                            self.scroll_offset = Some(new_offset);
                            self.scroll_jump_to = Some(new_offset);
                        } else {
                            // Currently at bottom, start scrolling up from max offset
                            let new_offset = (self.scroll_max_offset - 300.0).max(0.0);
                            self.scroll_offset = Some(new_offset);
                            self.scroll_jump_to = Some(new_offset);
                        }
                    } else {
                        // Scroll down (PageDown) - increase offset to show newer content
                        if let Some(offset) = self.scroll_offset {
                            let new_offset = offset + 300.0;
                            // If we're within one page of the bottom, snap to bottom
                            if new_offset >= self.scroll_max_offset - 10.0 {
                                self.scroll_offset = None;
                                self.scroll_jump_to = Some(self.scroll_max_offset);
                            } else {
                                self.scroll_offset = Some(new_offset);
                                self.scroll_jump_to = Some(new_offset);
                            }
                        }
                        // If scroll_offset is None, we're already at bottom, nothing to do
                    }
                }
            }

            // Handle filter popup escape
            if self.filter_active {
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::F4) {
                        self.filter_active = false;
                        self.filter_text.clear();
                    }
                });
            }

            // No top menu bar in gui2 - hamburger menu is in the status bar

            // Handle menu actions
            match action {
                Some("world_list") => {
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.world_list_selected = self.current_world;
                    self.only_connected_worlds = false;
                    self.popup_scroll_to_selected = true;
                }
                Some("connected_worlds") => {
                    self.open_connections_unified();
                }
                Some("world_selector") => {
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.world_list_selected = self.current_world;
                    self.only_connected_worlds = false;
                    self.popup_scroll_to_selected = true;
                }
                Some("actions") => {
                    self.open_actions_list_unified();
                }
                Some("edit_current") => self.open_world_editor(self.current_world),
                Some("setup") => self.popup_state = PopupState::Setup,
                Some("web") => self.popup_state = PopupState::Web,
                Some("font") => {
                    self.edit_font_name = self.font_name.clone();
                    self.edit_font_size = format!("{:.1}", self.font_size);
                    self.edit_font_scale = format!("{:.2}", self.font_scale);
                    self.edit_font_y_offset = format!("{:.2}", self.font_y_offset);
                    self.edit_font_baseline_offset = format!("{:.2}", self.font_baseline_offset);
                    self.popup_state = PopupState::Font;
                    self.popup_scroll_to_selected = true;
                }
                Some("font_changed") => {
                    // Font size was changed via S/M/L buttons - update server settings
                    self.update_global_settings();
                }
                Some("connect") => self.connect_world(self.current_world),
                Some("disconnect") => self.disconnect_world(self.current_world),
                Some("toggle_tags") => self.show_tags = !self.show_tags,
                Some("toggle_highlight") => self.highlight_actions = !self.highlight_actions,
                Some("resync") => {
                    // Request full state resync from server
                    if let Some(ref ws_tx) = self.ws_tx {
                        let _ = ws_tx.send(WsMessage::RequestState);
                    }
                }
                Some("spell_check") => {
                    if let Some(message) = self.handle_spell_check() {
                        // Add suggestion message to current world's output
                        if let Some(world) = self.worlds.get_mut(self.current_world) {
                            let seq = world.output_lines.len() as u64;
                            world.output_lines.push(TimestampedLine {
                                text: message,
                                ts: current_timestamp_secs(),
                                gagged: false,
                                from_server: false,
                                seq,
                                highlight_color: None,
                            });
                        }
                    }
                }
                Some("help") => self.popup_state = PopupState::Help,
                _ => {}
            }

            // Input area at bottom (full width, minimum 3 lines in gui2)
            let effective_input_height = self.input_height.max(3);
            let input_height = effective_input_height as f32 * (self.font_size * 1.3) + 8.0;
            let prompt_text = if self.current_world < self.worlds.len() {
                crate::util::strip_ansi_codes(&self.worlds[self.current_world].prompt)
            } else {
                String::new()
            };

            let input_bg = {
                let c = theme.bg();
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
            };
            egui::TopBottomPanel::bottom("input_panel")
                .exact_height(input_height)
                .frame(egui::Frame::none()
                    .fill(input_bg)
                    .inner_margin(egui::Margin { left: 3.0, right: 8.0, top: 2.0, bottom: 2.0 })
                    .stroke(egui::Stroke::NONE))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0; // Remove horizontal spacing

                    // Text input takes full area
                    // Build layout job with spell check coloring (misspelled words in red)
                    let input_id = egui::Id::new("main_input");
                    let misspelled = self.find_misspelled_words();
                    let font_id = egui::FontId::monospace(self.font_size);
                    let default_color = theme.fg();
                    let line_height = 16.0_f32; // Approximate line height for scrolling calc

                    // Build layouter using actual text parameter (not pre-computed job)
                    // This ensures cursor positioning works correctly when text changes
                    let misspelled_ranges = misspelled;
                    let layouter_font_id = font_id.clone();
                    let layouter_default_color = default_color;

                    // Calculate prompt width for offset
                    let prompt_width = if !prompt_text.is_empty() {
                        ui.fonts(|f| f.glyph_width(&egui::FontId::monospace(self.font_size), ' ')) * prompt_text.len() as f32
                    } else {
                        0.0
                    };

                    // Use ScrollArea to handle scrolling, with manual scroll-to-cursor
                    let available_height = ui.available_height();
                    let scroll_id = egui::Id::new("input_scroll_area");

                    let mut scroll_to_y: Option<f32> = None;

                    let scroll_output = egui::ScrollArea::vertical()
                        .id_source(scroll_id)
                        .max_height(available_height)
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            // Show prompt if present (cyan colored like TUI)
                            if !prompt_text.is_empty() {
                                ui.label(egui::RichText::new(&prompt_text)
                                    .monospace()
                                    .color(theme.prompt()));
                            }

                            let available_width = ui.available_width();
                            let response = ui.add(
                                TextEdit::multiline(&mut self.input_buffer)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(available_width)
                                    .margin(egui::vec2(0.0, 0.0))
                                    .frame(false)
                                    .id(input_id)
                                    .layouter(&mut |_ui, text, wrap_width| {
                                        // Build layout job from actual text parameter
                                        let mut job = egui::text::LayoutJob::default();
                                        let chars: Vec<char> = text.chars().collect();
                                        let mut pos = 0;

                                        // Use misspelled ranges (may be slightly stale for one frame)
                                        let mut ranges = misspelled_ranges.clone();
                                        ranges.sort_by_key(|(start, _)| *start);

                                        for (start, end) in ranges {
                                            if start >= chars.len() || end > chars.len() {
                                                continue; // Skip stale ranges
                                            }
                                            // Add normal text before misspelled word
                                            if pos < start {
                                                let normal_text: String = chars[pos..start].iter().collect();
                                                job.append(&normal_text, 0.0, egui::TextFormat {
                                                    font_id: layouter_font_id.clone(),
                                                    color: layouter_default_color,
                                                    ..Default::default()
                                                });
                                            }
                                            // Add misspelled word in red
                                            let misspelled_text: String = chars[start..end].iter().collect();
                                            job.append(&misspelled_text, 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: egui::Color32::RED,
                                                ..Default::default()
                                            });
                                            pos = end;
                                        }
                                        // Add remaining text
                                        if pos < chars.len() {
                                            let remaining: String = chars[pos..].iter().collect();
                                            job.append(&remaining, 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: layouter_default_color,
                                                ..Default::default()
                                            });
                                        }
                                        // Handle empty text
                                        if chars.is_empty() {
                                            job.append("", 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: layouter_default_color,
                                                ..Default::default()
                                            });
                                        }

                                        job.wrap = egui::text::TextWrapping {
                                            max_width: wrap_width,
                                            ..Default::default()
                                        };
                                        _ui.fonts(|f| f.layout_job(job))
                                    })
                            );
                            response
                        }).inner
                    });

                    let response = scroll_output.inner;

                    // Right-click context menu for input area
                    let input_id_for_menu = input_id;
                    let response = response.context_menu(|ui| {
                        #[cfg(feature = "arboard")]
                        if ui.button("Paste").clicked() {
                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                if let Ok(text) = clipboard.get_text() {
                                    // Insert clipboard text at cursor position
                                    if let Some(state) = egui::TextEdit::load_state(ui.ctx(), input_id_for_menu) {
                                        if let Some(cursor_range) = state.ccursor_range() {
                                            let cursor_pos = cursor_range.primary.index;
                                            let byte_pos: usize = self.input_buffer.char_indices()
                                                .nth(cursor_pos)
                                                .map(|(i, _)| i)
                                                .unwrap_or(self.input_buffer.len());
                                            self.input_buffer.insert_str(byte_pos, &text);
                                            // Move cursor after pasted text
                                            let new_cursor = cursor_pos + text.chars().count();
                                            let mut new_state = state;
                                            let ccursor = egui::text::CCursor::new(new_cursor);
                                            new_state.set_ccursor_range(Some(egui::text::CCursorRange::one(ccursor)));
                                            new_state.store(ui.ctx(), input_id_for_menu);
                                        }
                                    } else {
                                        // No cursor state - append to end
                                        self.input_buffer.push_str(&text);
                                    }
                                }
                            }
                            ui.close_menu();
                        }
                    });

                    // Check for temperature conversion when input changes
                    if response.changed() && self.check_temp_conversion() {
                        // Conversion happened - move cursor to end of buffer
                        let mut state = egui::TextEdit::load_state(ctx, input_id)
                            .unwrap_or_default();
                        let ccursor = egui::text::CCursor::new(self.input_buffer.chars().count());
                        state.set_ccursor_range(Some(egui::text::CCursorRange::one(ccursor)));
                        state.store(ctx, input_id);
                    }

                    // Calculate cursor position and scroll to it if needed
                    if response.has_focus() {
                        if let Some(state) = egui::TextEdit::load_state(ctx, input_id) {
                            if let Some(cursor_range) = state.ccursor_range() {
                                // Estimate cursor Y position based on character position
                                // Count newlines before cursor to estimate line number
                                let cursor_pos = cursor_range.primary.index;
                                let text_before_cursor: String = self.input_buffer.chars().take(cursor_pos).collect();
                                let lines_before = text_before_cursor.matches('\n').count();
                                // Also account for wrapped lines (rough estimate)
                                let wrap_width = ui.available_width() - prompt_width;
                                let char_width = ui.fonts(|f| f.glyph_width(&egui::FontId::monospace(self.font_size), 'M'));
                                let chars_per_line = (wrap_width / char_width).max(1.0) as usize;
                                let wrapped_lines: usize = text_before_cursor.lines()
                                    .map(|line| (line.len() / chars_per_line.max(1)).max(0))
                                    .sum();
                                let cursor_line = lines_before + wrapped_lines;
                                let cursor_y = cursor_line as f32 * line_height;

                                // Check if cursor is outside visible area
                                let scroll_offset = scroll_output.state.offset.y;
                                let visible_top = scroll_offset;
                                let visible_bottom = scroll_offset + available_height - line_height;

                                if cursor_y < visible_top {
                                    scroll_to_y = Some(cursor_y);
                                } else if cursor_y > visible_bottom {
                                    scroll_to_y = Some(cursor_y - available_height + line_height * 2.0);
                                }
                            }
                        }
                    }

                    // Apply scroll adjustment if needed
                    if let Some(target_y) = scroll_to_y {
                        let mut scroll_state = scroll_output.state;
                        scroll_state.offset.y = target_y.max(0.0);
                        scroll_state.store(ctx, scroll_output.id);
                    }

                    // Always keep cursor visible in input area when no popup is open
                    // But don't steal focus if user is selecting text with mouse
                    if !response.has_focus() && self.popup_state == PopupState::None && !self.filter_active {
                        let mouse_down = ctx.input(|i| i.pointer.any_down());
                        let typed_text: Option<String> = ctx.input(|i| {
                            // Find any text that was typed
                            for e in &i.events {
                                if let egui::Event::Text(text) = e {
                                    return Some(text.clone());
                                }
                            }
                            None
                        });

                        // Request focus if mouse isn't being held (not selecting text)
                        // or if user started typing
                        if !mouse_down || typed_text.is_some() {
                            response.request_focus();
                        }

                        if let Some(text) = typed_text {
                            // Add the typed text to input buffer
                            self.input_buffer.push_str(&text);
                            // Set cursor position to end of buffer (create state if needed)
                            let mut state = egui::TextEdit::load_state(ctx, input_id)
                                .unwrap_or_default();
                            let ccursor = egui::text::CCursor::new(self.input_buffer.len());
                            state.set_ccursor_range(Some(egui::text::CCursorRange::one(ccursor)));
                            state.store(ctx, input_id);
                        }
                    }

                    // Apply cursor_home (Ctrl+A)
                    if cursor_home {
                        let mut state = egui::TextEdit::load_state(ctx, input_id)
                            .unwrap_or_default();
                        let ccursor = egui::text::CCursor::new(0);
                        state.set_ccursor_range(Some(egui::text::CCursorRange::one(ccursor)));
                        state.store(ctx, input_id);
                    }

                    // Send on Enter (with or without Shift)
                    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        // Remove all newlines from the command (cursor in middle causes TextEdit to insert newline)
                        let cmd: String = std::mem::take(&mut self.input_buffer)
                            .chars()
                            .filter(|c| *c != '\n')
                            .collect();
                        // Reset spell state when sending command
                        self.reset_spell_state();
                        if !cmd.is_empty() {
                            // Use shared command parsing
                            let parsed = super::parse_command(&cmd);

                            // Handle local GUI popup commands
                            match parsed {
                                super::Command::Setup => {
                                    self.popup_state = PopupState::Setup;
                                }
                                super::Command::Web => {
                                    self.popup_state = PopupState::Web;
                                }
                                super::Command::WorldSelector => {
                                    self.popup_state = PopupState::ConnectedWorlds;
                                    self.world_list_selected = self.current_world;
                                    self.only_connected_worlds = false;
                                    self.popup_scroll_to_selected = true;
                                }
                                super::Command::WorldsList => {
                                    // Output connected worlds list as text
                                    let worlds_info: Vec<super::util::WorldListInfo> = self.worlds.iter().enumerate().map(|(idx, world)| {
                                        super::util::WorldListInfo {
                                            name: world.name.clone(),
                                            connected: world.connected,
                                            is_current: idx == self.current_world,
                                            is_ssl: world.settings.use_ssl,
                                            is_proxy: world.is_proxy,
                                            unseen_lines: world.unseen_lines,
                                            last_send_secs: world.last_send_secs,
                                            last_recv_secs: world.last_recv_secs,
                                            last_nop_secs: world.last_nop_secs,
                                            next_nop_secs: None,
                                            buffer_size: world.output_lines.len(),
                                        }
                                    }).collect();
                                    let output = super::util::format_worlds_list(&worlds_info);
                                    let ts = super::current_timestamp_secs();
                                    if self.current_world < self.worlds.len() {
                                        for line in output.lines() {
                                            let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                            self.worlds[self.current_world].output_lines.push(TimestampedLine {
                                                text: line.to_string(),
                                                ts,
                                                gagged: false,
                                                from_server: false,
                                                seq,
                                                highlight_color: None,
                                            });
                                        }
                                    }
                                }
                                super::Command::Help => {
                                    self.popup_state = PopupState::Help;
                                }
                                super::Command::Version => {
                                    let ts = current_timestamp_secs();
                                    if self.current_world < self.worlds.len() {
                                        let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                        self.worlds[self.current_world].output_lines.push(
                                            TimestampedLine { text: super::get_version_string(), ts, gagged: false, from_server: false, seq, highlight_color: None }
                                        );
                                    }
                                }
                                super::Command::Menu => {
                                    self.popup_state = PopupState::Menu;
                                    self.menu_selected = 0;
                                }
                                super::Command::Actions { .. } => {
                                    self.open_actions_list_unified();
                                }
                                super::Command::WorldEdit { name } => {
                                    // Open world editor
                                    let idx = if let Some(ref world_name) = name {
                                        self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(world_name))
                                            .unwrap_or(self.current_world)
                                    } else {
                                        self.current_world
                                    };
                                    self.open_world_editor(idx);
                                }
                                super::Command::WorldSwitch { ref name } => {
                                    // /worlds <name> - switch to world, connect if not connected
                                    if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                                        // Switch locally
                                        self.current_world = idx;
                                        self.selection_start = None; self.selection_end = None;
                                        // If not connected, send connect command to server
                                        if !self.worlds[idx].connected {
                                            self.connect_world(idx);
                                        }
                                    } else {
                                        // World not found - show error locally (red % prefix)
                                        let ts = current_timestamp_secs();
                                        if self.current_world < self.worlds.len() {
                                            let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                            self.worlds[self.current_world].output_lines.push(
                                                TimestampedLine { text: format!("✨ World '{}' not found.", name), ts, gagged: false, from_server: false, seq, highlight_color: None }
                                            );
                                        }
                                    }
                                }
                                super::Command::WorldConnectNoLogin { ref name } => {
                                    // /worlds -l <name> - switch to world, connect without auto-login
                                    if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                                        self.current_world = idx;
                                        self.selection_start = None; self.selection_end = None;
                                        if !self.worlds[idx].connected {
                                            // Send the command to server (it handles -l flag)
                                            self.send_command(idx, cmd);
                                        }
                                    } else {
                                        // World not found - show error locally (red % prefix)
                                        let ts = current_timestamp_secs();
                                        if self.current_world < self.worlds.len() {
                                            let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                            self.worlds[self.current_world].output_lines.push(
                                                TimestampedLine { text: format!("✨ World '{}' not found.", name), ts, gagged: false, from_server: false, seq, highlight_color: None }
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    // Check for /font which is GUI-specific
                                    if cmd.trim().eq_ignore_ascii_case("/font") {
                                        self.edit_font_name = self.font_name.clone();
                                        self.edit_font_size = format!("{:.1}", self.font_size);
                                        self.edit_font_scale = format!("{:.2}", self.font_scale);
                                        self.edit_font_y_offset = format!("{:.2}", self.font_y_offset);
                                        self.edit_font_baseline_offset = format!("{:.2}", self.font_baseline_offset);
                                        self.popup_state = PopupState::Font;
                                        self.popup_scroll_to_selected = true;
                                    } else {
                                        // Send other commands to server
                                        self.send_command(self.current_world, cmd);
                                    }
                                }
                            }
                        }
                    }

                    // Show suggestion message if present
                    if let Some(ref msg) = self.suggestion_message {
                        ui.label(egui::RichText::new(msg).color(theme.prompt()).monospace());
                    }
                });

            // Separator bar (gui2 redesign)
            let separator_bg = theme.status_bar_bg();
            let separator_bg_transparent = egui::Color32::from_rgba_unmultiplied(separator_bg.r(), separator_bg.g(), separator_bg.b(), alpha);
            egui::TopBottomPanel::bottom("separator_bar")
                .exact_height(27.0)
                .frame(egui::Frame::none()
                    .fill(separator_bg_transparent)
                    .inner_margin(egui::Margin::symmetric(10.0, 0.0))
                    .stroke(egui::Stroke::new(0.25, theme.border_medium())))
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;

                        // Scale separator bar font sizes relative to self.font_size (30% bigger)
                        let fs = self.font_size;
                        let fs_icon = fs * 1.3;         // hamburger icon
                        let fs_name = fs * 0.86 * 1.3;  // world name
                        let fs_tag = fs * 0.75 * 1.3;   // [tag] indicator
                        let fs_badge = fs * 0.79 * 1.3;  // MORE/HIST/ACT badges
                        let fs_time = fs * 0.79 * 1.3;   // time display
                        let fs_slider_label = fs * 0.57 * 1.3; // font slider labels

                        // Get current world info
                        let world_name = self.worlds.get(self.current_world)
                            .map(|w| w.name.as_str())
                            .unwrap_or("---");
                        let connected = self.worlds.get(self.current_world)
                            .map(|w| w.connected)
                            .unwrap_or(false);
                        let was_connected = self.worlds.get(self.current_world)
                            .map(|w| w.was_connected)
                            .unwrap_or(false);
                        // Compute activity locally excluding this client's current world
                        // (server_activity_count excludes the server's current world, which may differ)
                        let activity_count = self.worlds.iter().enumerate()
                            .filter(|(i, w)| *i != self.current_world && (w.unseen_lines > 0 || w.pending_count > 0))
                            .count();
                        let server_pending_count = self.worlds.get(self.current_world)
                            .map(|w| w.pending_count)
                            .unwrap_or(0);
                        let is_scrolled_back = self.scroll_offset.is_some();

                        // Hamburger menu button (shift right 8px)
                        ui.add_space(4.0);
                        let is_menu_open = self.hamburger_menu_open;
                        let btn_text_color = if is_menu_open { theme.accent() } else { theme.fg_muted() };
                        // 40% darker than separator bar bg for highlight
                        let hamburger_highlight = if theme.is_dark() {
                            let c = theme.status_bar_bg();
                            egui::Color32::from_rgb(
                                (c.r() as f32 * 0.6) as u8,
                                (c.g() as f32 * 0.6) as u8,
                                (c.b() as f32 * 0.6) as u8,
                            )
                        } else {
                            theme.bg_hover()
                        };
                        // Override hover/active styles for this button
                        ui.visuals_mut().widgets.hovered.bg_fill = hamburger_highlight;
                        ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                        ui.visuals_mut().widgets.active.bg_fill = hamburger_highlight;
                        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
                        let menu_btn = ui.add(egui::Button::new(
                            egui::RichText::new("☰").size(fs_icon).color(btn_text_color)
                        ).min_size(egui::vec2(30.0, 30.0))
                         .rounding(egui::Rounding::same(4.0))
                         .fill(if is_menu_open { hamburger_highlight } else { egui::Color32::TRANSPARENT })
                         .stroke(egui::Stroke::NONE));
                        // Restore default widget styles
                        ui.visuals_mut().widgets.hovered.bg_fill = theme.selection_bg();
                        ui.visuals_mut().widgets.active.bg_fill = theme.selection_bg();

                        if menu_btn.clicked() {
                            self.hamburger_menu_open = !self.hamburger_menu_open;
                            if self.hamburger_menu_open {
                                self.hamburger_opened_time = std::time::Instant::now();
                            }
                            ctx.request_repaint();
                        }

                        // Connection dot + world name
                        if was_connected {
                            let ball_color = if connected { theme.success() } else { theme.error() };
                            let (dot_rect, _) = ui.allocate_exact_size(
                                egui::vec2(9.0, 9.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot_rect.center(), 4.5, ball_color);

                            ui.label(egui::RichText::new(world_name)
                                .strong().color(theme.fg()).size(fs_name));

                            if self.show_tags {
                                ui.label(egui::RichText::new("[tag]")
                                    .color(theme.accent()).size(fs_tag));
                            }
                            if self.worlds.get(self.current_world).map_or(false, |w| w.gmcp_user_enabled) {
                                ui.label(egui::RichText::new("[g]")
                                    .color(theme.accent()).size(fs_tag));
                            }
                        }

                        // More/Hist indicator badge at fixed position
                        // Reserve space for: hamburger(30+4px) + dot(9px) + spacing + 15 chars + 20px gap
                        let char_width = fs_name * 0.6; // approximate monospace char width
                        let target_x = 4.0 + 30.0 + ui.spacing().item_spacing.x + 9.0 + ui.spacing().item_spacing.x + (15.0 * char_width) + 20.0;
                        let current_x = ui.cursor().left() - ui.min_rect().left();
                        if target_x > current_x {
                            ui.add_space(target_x - current_x);
                        } else {
                            ui.add_space(20.0);
                        }

                        if is_scrolled_back {
                            let lines_back = self.scroll_offset
                                .map(|offset| ((self.scroll_max_offset - offset).max(0.0) / 20.0).max(1.0) as usize)
                                .unwrap_or(0);
                            let count_str = if lines_back >= 10000 {
                                format!("{}K", (lines_back / 1000).min(999))
                            } else {
                                format!("{}", lines_back)
                            };
                            let hist_bg = egui::Color32::from_rgb(0xcc, 0x1a, 0x0e);
                            let hist_num_bg = egui::Color32::from_rgb(0xa1, 0x0b, 0x00);
                            let hist_text_color = egui::Color32::WHITE;
                            let badge_height = fs_badge * 1.6;
                            ui.horizontal_centered(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let r1 = ui.add(egui::Button::new(
                                    egui::RichText::new(" History ").monospace().size(fs_badge).strong()
                                        .color(hist_text_color))
                                    .fill(hist_bg)
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 4.0, sw: 4.0, ne: 0.0, se: 0.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                let r2 = ui.add(egui::Button::new(
                                    egui::RichText::new(format!(" {} ", count_str)).monospace().size(fs_badge).strong()
                                        .color(hist_text_color))
                                    .fill(hist_num_bg)
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 0.0, sw: 0.0, ne: 4.0, se: 4.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                if r1.clicked() || r2.clicked() {
                                    // Scroll to bottom (same as PageDown when viewing history)
                                    self.scroll_offset = None;
                                    self.scroll_jump_to = Some(self.scroll_max_offset);
                                }
                            });
                        } else if server_pending_count > 0 {
                            let count_str = if server_pending_count >= 1_000_000 {
                                "Alot".to_string()
                            } else if server_pending_count >= 10000 {
                                format!("{}K", (server_pending_count / 1000).min(999))
                            } else {
                                format!("{}", server_pending_count)
                            };
                            let more_bg = egui::Color32::from_rgb(0xcc, 0x1a, 0x0e);
                            let more_num_bg = egui::Color32::from_rgb(0xa1, 0x0b, 0x00);
                            let more_text_color = egui::Color32::WHITE;
                            let badge_height = fs_badge * 1.6;
                            ui.horizontal_centered(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let r1 = ui.add(egui::Button::new(
                                    egui::RichText::new(" More ").monospace().size(fs_badge).strong()
                                        .color(more_text_color))
                                    .fill(more_bg)
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 4.0, sw: 4.0, ne: 0.0, se: 0.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                let r2 = ui.add(egui::Button::new(
                                    egui::RichText::new(format!(" {} ", count_str)).monospace().size(fs_badge).strong()
                                        .color(more_text_color))
                                    .fill(more_num_bg)
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 0.0, sw: 0.0, ne: 4.0, se: 4.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                if r1.clicked() || r2.clicked() {
                                    // Release one screenful of pending lines (same as Tab)
                                    let release_count = self.output_visible_lines.saturating_sub(2).max(1);
                                    if let Some(ref tx) = self.ws_tx {
                                        let _ = tx.send(WsMessage::ReleasePending {
                                            world_index: self.current_world,
                                            count: release_count,
                                        });
                                    }
                                }
                            });
                        }

                        // Activity indicator badge (right after More/Hist)
                        if activity_count > 0 {
                            ui.add_space(8.0);
                            let badge_height = fs_badge * 1.6;
                            let act_response = ui.horizontal_centered(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let r1 = ui.add(egui::Button::new(
                                    egui::RichText::new(" Activity ").monospace().size(fs_badge).strong()
                                        .color(egui::Color32::BLACK))
                                    .fill(theme.activity_label_bg())
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 4.0, sw: 4.0, ne: 0.0, se: 0.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                let r2 = ui.add(egui::Button::new(
                                    egui::RichText::new(format!(" {} ", activity_count)).monospace().size(fs_badge).strong()
                                        .color(egui::Color32::BLACK))
                                    .fill(theme.activity_count_bg())
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding {
                                        nw: 0.0, sw: 0.0, ne: 4.0, se: 4.0,
                                    })
                                    .min_size(egui::vec2(0.0, badge_height))
                                    .sense(egui::Sense::click()));
                                if r1.clicked() || r2.clicked() {
                                    // Switch to world with activity (same as Down arrow)
                                    if let Some(ref tx) = self.ws_tx {
                                        let _ = tx.send(WsMessage::CalculateNextWorld { current_index: self.current_world });
                                    }
                                }
                            }).response;
                            act_response.on_hover_ui(|ui| {
                                ui.label(egui::RichText::new("Worlds with activity:").strong());
                                for (i, w) in self.worlds.iter().enumerate() {
                                    if i != self.current_world && (w.unseen_lines > 0 || w.pending_count > 0) {
                                        if w.pending_count > 0 && w.unseen_lines > 0 {
                                            ui.label(format!("{}: {} unseen, {} pending", w.name, w.unseen_lines, w.pending_count));
                                        } else if w.pending_count > 0 {
                                            ui.label(format!("{}: {} pending", w.name, w.pending_count));
                                        } else {
                                            ui.label(format!("{}: {} unseen", w.name, w.unseen_lines));
                                        }
                                    }
                                }
                            });
                        }

                        // Right side: time, font slider (RTL order)
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Time (rightmost)
                            let lt = local_time_now();
                            let hours_24 = lt.hour as u32;
                            let mins = lt.minute as u32;
                            let hours = if hours_24 == 0 { 12 }
                                else if hours_24 <= 12 { hours_24 }
                                else { hours_24 - 12 };
                            let time_color = theme.fg();
                            ui.label(egui::RichText::new(format!("{}:{:02}", hours, mins))
                                .monospace().size(fs_time).color(time_color));

                            ui.add_space(15.0);

                            // Custom-painted font size slider (matches mockup: 60px track, 4px tall, 12px circle thumb)
                            let label_color = if theme.is_dark() { theme.fg_muted() } else { theme.fg() };
                            ui.label(egui::RichText::new(format!("{:.0}", self.font_size))
                                .monospace().size(fs_slider_label).color(label_color));

                            // Custom slider widget
                            let slider_width = 60.0_f32;
                            let slider_total_height = 14.0_f32; // enough room for 12px thumb
                            let track_height = 4.0_f32;
                            let thumb_radius = 6.0_f32;
                            let (slider_rect, slider_resp) = ui.allocate_exact_size(
                                egui::vec2(slider_width, slider_total_height),
                                egui::Sense::click_and_drag(),
                            );

                            // Handle interaction
                            if slider_resp.dragged() || slider_resp.clicked() {
                                if let Some(pos) = slider_resp.interact_pointer_pos() {
                                    let t = ((pos.x - slider_rect.left() - thumb_radius) / (slider_width - thumb_radius * 2.0)).clamp(0.0, 1.0);
                                    let new_val = 9.0 + t * 11.0; // range 9..=20
                                    self.font_size = new_val.round();
                                    action = Some("font_changed");
                                }
                            }

                            // Paint track
                            let track_color = if theme.is_dark() { theme.border_medium() } else { egui::Color32::from_rgb(0x88, 0x88, 0x88) };
                            let track_y = slider_rect.center().y;
                            let track_rect = egui::Rect::from_min_max(
                                egui::pos2(slider_rect.left() + thumb_radius, track_y - track_height / 2.0),
                                egui::pos2(slider_rect.right() - thumb_radius, track_y + track_height / 2.0),
                            );
                            ui.painter().rect_filled(track_rect, egui::Rounding::same(2.0), track_color);

                            // Paint thumb
                            let t = ((self.font_size - 9.0) / 11.0).clamp(0.0, 1.0);
                            let thumb_x = track_rect.left() + t * track_rect.width();
                            let thumb_color = if slider_resp.hovered() || slider_resp.dragged() {
                                theme.accent()
                            } else {
                                theme.fg_muted()
                            };
                            let thumb_border = if theme.is_dark() { theme.border_medium() } else { egui::Color32::from_rgb(0x77, 0x77, 0x77) };
                            ui.painter().circle_filled(egui::pos2(thumb_x, track_y), thumb_radius, thumb_color);
                            ui.painter().circle_stroke(egui::pos2(thumb_x, track_y), thumb_radius, egui::Stroke::new(1.5, thumb_border));

                            ui.label(egui::RichText::new("A")
                                .size(fs_slider_label).color(label_color));
                        });
                    });
                });

            // Hamburger menu popup (gui2)
            if self.hamburger_menu_open {
                let menu_bg = theme.menu_bar_bg();
                // Position menu: bottom edge at separator bar top, left edge at hamburger button
                let bar_top = ctx.screen_rect().height() - 34.0 - input_height;
                let menu_width = 195.0;
                // 9 items × 24px + 3 separators × 6px + 12px inner margin padding
                let menu_height = 9.0 * 24.0 + 3.0 * 6.0 + 12.0;
                let menu_pos = egui::pos2(8.0, (bar_top - menu_height + 2.0).max(2.0));

                let mut close_menu = false;
                egui::Window::new("##hamburger_menu")
                    .fixed_pos(menu_pos)
                    .fixed_size(egui::vec2(menu_width, menu_height))
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .frame(egui::Frame::none()
                        .fill(menu_bg)
                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(6.0)))
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;

                        let row_width = menu_width - 12.0;
                        let clicked = |ui: &mut egui::Ui, label: &str, shortcut: &str| -> bool {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(row_width, 24.0),
                                egui::Sense::click(),
                            );
                            if ui.is_rect_visible(rect) {
                                // Hover highlight
                                if response.hovered() {
                                    ui.painter().rect_filled(rect, 3.0, theme.bg_hover());
                                }
                                // Label left-aligned
                                ui.painter().text(
                                    egui::pos2(rect.left() + 8.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    label,
                                    egui::FontId::proportional(12.0),
                                    theme.fg(),
                                );
                                // Shortcut right-aligned
                                if !shortcut.is_empty() {
                                    ui.painter().text(
                                        egui::pos2(rect.right() - 8.0, rect.center().y),
                                        egui::Align2::RIGHT_CENTER,
                                        shortcut,
                                        egui::FontId::monospace(10.0),
                                        theme.fg_muted(),
                                    );
                                }
                            }
                            response.clicked()
                        };

                        if clicked(ui, "Worlds", "") { action = Some("world_selector"); close_menu = true; }
                        if clicked(ui, "World Editor", "Ctrl+E") { action = Some("edit_current"); close_menu = true; }
                        if clicked(ui, "Actions", "") { action = Some("actions"); close_menu = true; }
                        ui.separator();
                        if clicked(ui, "Settings", "Ctrl+S") { action = Some("setup"); close_menu = true; }
                        if clicked(ui, "Web Settings", "") { action = Some("web"); close_menu = true; }
                        if clicked(ui, "Font", "") { action = Some("font"); close_menu = true; }
                        ui.separator();
                        if clicked(ui, "Toggle Tags", "F2") { action = Some("toggle_tags"); close_menu = true; }
                        if clicked(ui, "Search", "F4") { self.filter_active = !self.filter_active; close_menu = true; }
                        ui.separator();
                        if clicked(ui, "Resync", "") { action = Some("resync"); close_menu = true; }
                    });

                if close_menu {
                    self.hamburger_menu_open = false;
                    ctx.request_repaint();
                }

                // Close on Escape
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.hamburger_menu_open = false;
                    ctx.request_repaint();
                }

                // Close on click outside (skip first 200ms to avoid fighting with open click)
                let elapsed = self.hamburger_opened_time.elapsed();
                if elapsed.as_millis() > 100 && ctx.input(|i| i.pointer.any_pressed()) {
                    if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                        let menu_rect = egui::Rect::from_min_size(menu_pos, egui::vec2(menu_width + 10.0, menu_height));
                        let btn_rect = egui::Rect::from_min_size(
                            egui::pos2(0.0, bar_top), egui::vec2(40.0, 34.0));
                        if !menu_rect.contains(pos) && !btn_rect.contains(pos) {
                            self.hamburger_menu_open = false;
                            ctx.request_repaint();
                        }
                    }
                }
            }

            // Handle menu actions set by hamburger menu or separator bar
            match action {
                Some("world_list") => {
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.world_list_selected = self.current_world;
                    self.only_connected_worlds = false;
                    self.popup_scroll_to_selected = true;
                }
                Some("connected_worlds") => {
                    self.open_connections_unified();
                }
                Some("world_selector") => {
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.world_list_selected = self.current_world;
                    self.only_connected_worlds = false;
                    self.popup_scroll_to_selected = true;
                }
                Some("actions") => {
                    self.open_actions_list_unified();
                }
                Some("edit_current") => self.open_world_editor(self.current_world),
                Some("setup") => self.popup_state = PopupState::Setup,
                Some("web") => self.popup_state = PopupState::Web,
                Some("font") => {
                    self.edit_font_name = self.font_name.clone();
                    self.edit_font_size = format!("{:.1}", self.font_size);
                    self.edit_font_scale = format!("{:.2}", self.font_scale);
                    self.edit_font_y_offset = format!("{:.2}", self.font_y_offset);
                    self.edit_font_baseline_offset = format!("{:.2}", self.font_baseline_offset);
                    self.popup_state = PopupState::Font;
                    self.popup_scroll_to_selected = true;
                }
                Some("font_changed") => {
                    self.update_global_settings();
                }
                Some("connect") => self.connect_world(self.current_world),
                Some("disconnect") => self.disconnect_world(self.current_world),
                Some("toggle_tags") => self.show_tags = !self.show_tags,
                Some("toggle_highlight") => self.highlight_actions = !self.highlight_actions,
                Some("resync") => {
                    if let Some(ref ws_tx) = self.ws_tx {
                        let _ = ws_tx.send(WsMessage::RequestState);
                    }
                }
                Some("redraw") => {
                    // Filter output to only server data (remove client-generated lines)
                    if let Some(world) = self.worlds.get_mut(self.current_world) {
                        world.output_lines.retain(|line| line.from_server);
                    }
                    // Also request a full resync from server
                    if let Some(ref ws_tx) = self.ws_tx {
                        let _ = ws_tx.send(WsMessage::RequestState);
                    }
                }
                Some("help") => self.popup_state = PopupState::Help,
                _ => {}
            }

            // Main output area with scrollbar (no frame/border/margin)
            let bg = {
                let c = theme.bg();
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
            };
            egui::CentralPanel::default()
                .frame(egui::Frame::none()
                    .fill(bg)
                    .inner_margin(egui::Margin { left: 0.0, right: 0.0, top: 7.0, bottom: 0.0 })
                    .stroke(egui::Stroke::NONE))
                .show(ctx, |ui| {
                // Clip output area so scrollbar doesn't bleed into separator bar
                ui.set_clip_rect(ui.max_rect());
                if let Some(world) = self.worlds.get(self.current_world) {
                    // Check if showing splash screen - render centered image instead of output
                    if world.showing_splash {
                        // Center the splash content vertically and horizontally
                        ui.vertical_centered(|ui| {
                            let available_height = ui.available_height();
                            // Calculate vertical centering (image ~200px + text ~60px)
                            let content_height = 280.0;
                            let top_padding = (available_height - content_height).max(0.0) / 2.0;
                            ui.add_space(top_padding);

                            // Clay logo image
                            let splash_image = egui::Image::from_bytes(
                                "bytes://clay_splash",
                                include_bytes!("../clay2.png"),
                            ).fit_to_exact_size(egui::vec2(200.0, 200.0));
                            ui.add(splash_image);

                            let tagline_color = egui::Color32::from_rgb(0xff, 0x87, 0xff);  // 213
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("A 90dies mud client written today").color(tagline_color).italics());
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("/help for how to use clay").color(theme.fg_muted()));
                        });
                        return;  // Skip regular output rendering
                    }

                    // Check if output needs rebuilding by detecting any state change
                    let available_width = ui.available_width();
                    let current_line_count = world.output_lines.len();
                    if current_line_count != self.cached_output_len
                        || self.current_world != self.cached_world_index
                        || self.show_tags != self.cached_show_tags
                        || self.highlight_actions != self.cached_highlight_actions
                        || self.filter_text != self.cached_filter_text
                        || self.font_size != self.cached_font_size
                        || self.color_offset_percent != self.cached_color_offset
                        || (available_width - self.cached_output_width).abs() > 1.0 {
                        self.output_dirty = true;
                    }

                    let default_color = theme.fg();
                    let font_id = egui::FontId::monospace(self.font_size);
                    let is_light_theme = !theme.is_dark();

                    if self.output_dirty {
                        // Cache "now" for timestamp formatting - compute once per frame
                        let cached_now = GuiCachedNow::new();

                        const MAX_RENDER_LINES: usize = 2000;

                        // Keep original lines with ANSI for coloring
                        let filtering = self.filter_active && !self.filter_text.is_empty();

                        // Pre-truncate to avoid iterating thousands of lines when not filtering
                        let lines_slice: &[TimestampedLine] = if !filtering && world.output_lines.len() > MAX_RENDER_LINES + 100 {
                            &world.output_lines[world.output_lines.len() - MAX_RENDER_LINES - 100..]
                        } else {
                            &world.output_lines
                        };

                        // Pre-compile filter regex once (not per-line)
                        let filter_regex = if filtering {
                            let has_wildcards = self.filter_text.contains('*') || self.filter_text.contains('?');
                            if has_wildcards {
                                filter_wildcard_to_regex(&self.filter_text)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        let filter_lower = if filtering { self.filter_text.to_lowercase() } else { String::new() };

                        let colored_lines: Vec<&TimestampedLine> = lines_slice.iter()
                            .filter(|line| {
                                // Skip gagged lines unless show_tags is enabled (F2)
                                if line.gagged && !self.show_tags {
                                    return false;
                                }
                                // Apply filter if active (filter on stripped text)
                                if filtering {
                                    let stripped = crate::util::strip_ansi_codes(&line.text);
                                    if let Some(ref regex) = filter_regex {
                                        regex.is_match(&stripped)
                                    } else {
                                        // Simple substring match
                                        stripped.to_lowercase().contains(&filter_lower)
                                    }
                                } else {
                                    true
                                }
                            })
                            .collect();

                        // Cap rendered lines
                        let colored_lines = if !filtering && colored_lines.len() > MAX_RENDER_LINES {
                            colored_lines[colored_lines.len() - MAX_RENDER_LINES..].to_vec()
                        } else {
                            colored_lines
                        };

                        // Build plain text version for selection (strip ANSI codes and empty lines)
                        let lines: Vec<String> = colored_lines.iter()
                            .map(|line| {
                                let stripped = crate::util::strip_ansi_codes(&line.text);
                                if self.show_tags {
                                    // Add timestamp prefix when showing tags
                                    // Convert temperatures only if enabled
                                    let ts_prefix = Self::format_timestamp_gui_cached(line.ts, &cached_now);
                                    let with_temps = if self.temp_convert_enabled {
                                        convert_temperatures(&stripped)
                                    } else {
                                        stripped.clone()
                                    };
                                    format!("{} {}", ts_prefix, with_temps)
                                } else {
                                    // Strip MUD tags like [channel:] or [channel(player)]
                                    Self::strip_mud_tags(&stripped)
                                }
                            })
                            .collect();
                        let plain_text: String = lines.join("\n");

                        // Build combined LayoutJob with ANSI colors
                        let mut combined_job = egui::text::LayoutJob {
                            wrap: egui::text::TextWrapping {
                                max_width: available_width,
                                ..Default::default()
                            },
                            ..Default::default()
                        };

                        // Check if any line has Discord emojis (skip when show_tags enabled to show original text)
                        let has_any_discord_emojis = !self.show_tags && colored_lines.iter()
                            .any(|line| Self::has_discord_emojis(&line.text));

                        // Build display lines for both paths
                        let world_name = &world.name;
                        let highlight_actions = self.highlight_actions;
                        // Pre-compile action patterns once (not per-line)
                        let compiled_patterns = if highlight_actions {
                            compile_action_patterns(world_name, &self.actions)
                        } else {
                            Vec::new()
                        };
                        let display_lines: Vec<String> = colored_lines.iter().map(|line| {
                            let base_line = if self.show_tags {
                                let ts_prefix = Self::format_timestamp_gui_cached(line.ts, &cached_now);
                                // Convert temperatures only if enabled
                                let with_temps = if self.temp_convert_enabled {
                                    convert_temperatures(&line.text)
                                } else {
                                    line.text.clone()
                                };
                                format!("\x1b[36m{}\x1b[0m {}", ts_prefix, with_temps)
                            } else {
                                Self::strip_mud_tags_ansi(&line.text)
                            };
                            // Colorize ✨ emoji yellow (matches console terminal appearance)
                            let base_line = base_line.replace("✨", "\x1b[33m✨\x1b[0m");
                            // Apply /highlight color from action command (takes priority)
                            if let Some(ref hl_color) = line.highlight_color {
                                let bg_code = Self::color_name_to_ansi_bg(hl_color);
                                // Replace any resets to preserve background
                                let highlighted = base_line.replace("\x1b[0m", &format!("\x1b[0m{}", bg_code));
                                format!("{}{}\x1b[0m", bg_code, highlighted)
                            }
                            // Apply F8 action highlighting if enabled (and no explicit highlight color)
                            else if highlight_actions && line_matches_compiled_patterns(&line.text, &compiled_patterns) {
                                // Dark yellow/brown background (same as console: 48;5;58)
                                let bg_code = "\x1b[48;5;58m";
                                // Replace any resets to preserve background
                                let highlighted = base_line.replace("\x1b[0m", &format!("\x1b[0m{}", bg_code));
                                format!("{}{}\x1b[0m", bg_code, highlighted)
                            } else {
                                base_line
                            }
                        }).collect();

                        // Build combined LayoutJob from display_lines
                        if !has_any_discord_emojis {
                            for (i, display_line) in display_lines.iter().enumerate() {
                                // Skip Discord emoji conversion when show_tags enabled to show original text
                                let line_text = if self.show_tags {
                                    display_line.clone()
                                } else {
                                    convert_discord_emojis(display_line)
                                };
                                // Apply word breaks for long words
                                let line_text = Self::insert_word_breaks(&line_text);
                                Self::append_ansi_to_job(&line_text, default_color, font_id.clone(), &mut combined_job, is_light_theme, self.color_offset_percent, self.blink_visible);

                                if i < display_lines.len() - 1 {
                                    combined_job.append("\n", 0.0, egui::TextFormat {
                                        font_id: font_id.clone(),
                                        color: default_color,
                                        ..Default::default()
                                    });
                                }
                            }
                        }

                        // Cache the results
                        self.cached_output_job = Some(combined_job);
                        self.cached_plain_text = plain_text;
                        self.has_blink_text = display_lines.iter().any(|line| {
                            line.contains("\x1b[5m") || line.contains("\x1b[6m")
                                || line.contains(";5m") || line.contains(";6m")
                                || line.contains(";5;") || line.contains(";6;")
                        });
                        self.cached_display_lines = display_lines;
                        self.cached_has_emojis = has_any_discord_emojis;
                        self.cached_output_width = available_width;
                        self.cached_output_len = current_line_count;
                        self.cached_world_index = self.current_world;
                        self.cached_show_tags = self.show_tags;
                        self.cached_highlight_actions = self.highlight_actions;
                        self.cached_filter_text = self.filter_text.clone();
                        self.cached_font_size = self.font_size;
                        self.cached_color_offset = self.color_offset_percent;
                        self.urls_dirty = true;
                        self.output_dirty = false;
                    }

                    // Calculate approximate visible lines based on available height and font size
                    // This is used for more-mode triggering
                    let line_height = self.font_size * 1.3; // Approximate line height with some spacing
                    let available_height_for_output = ui.available_height();
                    let estimated_visible_lines = (available_height_for_output / line_height).max(5.0) as usize;
                    self.output_visible_lines = estimated_visible_lines;

                    // Send UpdateViewState if view changed (world or visible lines)
                    let current_state = (self.current_world, self.output_visible_lines);
                    if self.last_sent_view_state != Some(current_state) {
                        if let Some(ref tx) = self.ws_tx {
                            let _ = tx.send(WsMessage::UpdateViewState {
                                world_index: self.current_world,
                                visible_lines: self.output_visible_lines,
                            });
                            self.last_sent_view_state = Some(current_state);
                        }
                    }

                    // Use a unique ID per world to ensure scroll state is preserved per-world
                    let scroll_id = egui::Id::new(format!("output_scroll_{}", self.current_world));
                    let stick_to_bottom = self.scroll_offset.is_none() && !self.filter_active;

                    // Apply scroll jump if set (one-time from PageUp/PageDown)
                    let scroll_delta = self.scroll_jump_to.take();

                    let mut scroll_area = ScrollArea::vertical()
                        .id_source(scroll_id)
                        .auto_shrink([false; 2])
                        .stick_to_bottom(stick_to_bottom)
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);

                    // If we have a scroll delta, apply it
                    if let Some(delta) = scroll_delta {
                        scroll_area = scroll_area.vertical_scroll_offset(delta);
                    }

                    // Use cached values for rendering
                    let layout_job = self.cached_output_job.clone().unwrap_or_default();

                    let has_emojis = self.cached_has_emojis;
                    let emoji_lines = self.cached_display_lines.clone();
                    let emoji_font_id = font_id.clone();
                    let emoji_default_color = default_color;
                    let emoji_is_light = is_light_theme;
                    let emoji_link_color = theme.link();
                    let emoji_plain_text = self.cached_plain_text.clone();
                    let emoji_color_offset = self.color_offset_percent;
                    let emoji_blink_visible = self.blink_visible;

                    let scroll_output = scroll_area.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // Remove vertical spacing between widgets
                            ui.spacing_mut().item_spacing.y = 0.0;

                            // Use different rendering path for Discord emojis
                            if has_emojis {
                                // Render each line with inline emoji images
                                for line in &emoji_lines {
                                    Self::render_line_with_emojis(
                                        ui,
                                        line,
                                        emoji_default_color,
                                        &emoji_font_id,
                                        emoji_is_light,
                                        emoji_link_color,
                                        emoji_color_offset,
                                        emoji_blink_visible,
                                    );
                                }

                                // Add context menu for copy functionality
                                // Use Sense::hover() instead of Sense::click() to avoid capturing primary clicks
                                // that should go to the Link widgets for URL clicking
                                let text_for_copy = emoji_plain_text.clone();
                                ui.interact(ui.min_rect(), egui::Id::new("emoji_output_ctx"), egui::Sense::hover())
                                    .context_menu(|ui| {
                                        if ui.button("Copy All").clicked() {
                                            ui.ctx().copy_text(text_for_copy.clone());
                                            ui.close_menu();
                                        }
                                    });

                                return; // Skip the TextEdit path
                            }

                            // Custom rendering: paint backgrounds first, then text
                            // This ensures backgrounds fill full row height without gaps

                            // Layout the text WITH TRANSPARENT BACKGROUNDS
                            // We'll paint our own full-height backgrounds using bg_color_map
                            let mut job = layout_job.clone();
                            job.wrap.max_width = ui.available_width();
                            // Clear backgrounds from the job so galley won't paint short backgrounds
                            for section in &mut job.sections {
                                section.format.background = egui::Color32::TRANSPARENT;
                            }
                            let galley = ui.fonts(|f| f.layout_job(job));

                            // Allocate space for the galley
                            let (alloc_response, painter) = ui.allocate_painter(
                                galley.size(),
                                egui::Sense::click_and_drag()
                            );
                            let text_pos = alloc_response.rect.min;

                            // Handle text selection with mouse
                            let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                            // Get the position where the press started (not current position)
                            let press_origin = ui.input(|i| i.pointer.press_origin());

                            // Handle selection start on primary click
                            if alloc_response.drag_started_by(egui::PointerButton::Primary) {
                                // Use press_origin for accurate selection start position
                                if let Some(pos) = press_origin {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    self.selection_start = Some(cursor.ccursor.index);
                                    self.selection_end = Some(cursor.ccursor.index);
                                    self.selection_dragging = true;
                                }
                            }

                            // Double-click to select word
                            if alloc_response.double_clicked() {
                                if let Some(pos) = press_origin {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    let idx = cursor.ccursor.index;
                                    let text = galley.text();
                                    let chars: Vec<char> = text.chars().collect();
                                    // Walk backward to find word start
                                    let mut word_start = idx;
                                    while word_start > 0 && chars.get(word_start - 1).is_some_and(|c| !c.is_whitespace() && !c.is_ascii_punctuation()) {
                                        word_start -= 1;
                                    }
                                    // Walk forward to find word end
                                    let mut word_end = idx;
                                    while word_end < chars.len() && chars.get(word_end).is_some_and(|c| !c.is_whitespace() && !c.is_ascii_punctuation()) {
                                        word_end += 1;
                                    }
                                    if word_start != word_end {
                                        self.selection_start = Some(word_start);
                                        self.selection_end = Some(word_end);
                                        self.selection_dragging = false;
                                    }
                                }
                            }

                            // Triple-click to select line
                            if alloc_response.triple_clicked() {
                                if let Some(pos) = press_origin {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    let row_idx = cursor.rcursor.row;
                                    let rows = &galley.rows;
                                    if row_idx < rows.len() {
                                        // Find character offset at start of this row
                                        let row_start = galley.from_rcursor(egui::epaint::text::cursor::RCursor { row: row_idx, column: 0 });
                                        // Find character offset at end of this row
                                        let row = &rows[row_idx];
                                        let row_end_col = row.glyphs.len();
                                        let row_end = galley.from_rcursor(egui::epaint::text::cursor::RCursor { row: row_idx, column: row_end_col });
                                        self.selection_start = Some(row_start.ccursor.index);
                                        self.selection_end = Some(row_end.ccursor.index);
                                        self.selection_dragging = false;
                                    }
                                }
                            }

                            // Clear selection on single click without drag
                            if alloc_response.clicked_by(egui::PointerButton::Primary) && !self.selection_dragging {
                                self.selection_start = None;
                                self.selection_end = None;
                            }

                            // Handle selection update during drag
                            if self.selection_dragging && alloc_response.dragged_by(egui::PointerButton::Primary) {
                                if let Some(pos) = pointer_pos {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    self.selection_end = Some(cursor.ccursor.index);
                                }
                            }

                            // Handle selection end on release
                            if alloc_response.drag_released() {
                                self.selection_dragging = false;
                            }

                            // Clear selection when clicking outside the output area
                            if alloc_response.clicked_elsewhere() {
                                self.selection_start = None;
                                self.selection_end = None;
                                let sel_id = egui::Id::new(format!("output_selection_{}", self.current_world));
                                let sel_range_id = egui::Id::new(format!("output_selection_range_{}", self.current_world));
                                let sel_raw_id = egui::Id::new(format!("output_selection_raw_{}", self.current_world));
                                ui.ctx().data_mut(|d| {
                                    d.remove::<String>(sel_id);
                                    d.remove::<(usize, usize)>(sel_range_id);
                                    d.remove::<String>(sel_raw_id);
                                });
                            }

                            // Cache URLs from galley text (only recompute when output changed)
                            if self.urls_dirty {
                                self.cached_urls = Self::find_urls(galley.text());
                                self.urls_dirty = false;
                            }
                            let url_ranges = &self.cached_urls;

                            // Handle URL clicks - on single click (not drag), check if clicking on URL
                            if alloc_response.clicked_by(egui::PointerButton::Primary) {
                                if let Some(pos) = pointer_pos {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    let click_char = cursor.ccursor.index;

                                    for (start, end, url) in url_ranges {
                                        if click_char >= *start && click_char < *end {
                                            // Strip zero-width spaces that were inserted for word breaking
                                            let clean_url: String = url.replace('\u{200B}', "");
                                            Self::open_url(&clean_url);
                                            break;
                                        }
                                    }
                                }
                            }

                            // Show pointer cursor when hovering over URLs, I-beam over output text
                            let hover_pos = ui.input(|i| i.pointer.hover_pos());
                            if let Some(pos) = hover_pos {
                                if alloc_response.rect.contains(pos) {
                                    let relative_pos = pos - text_pos;
                                    let cursor = galley.cursor_from_pos(relative_pos);
                                    let hover_char = cursor.ccursor.index;

                                    let mut over_url = false;
                                    for (start, end, _) in url_ranges {
                                        if hover_char >= *start && hover_char < *end {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                            over_url = true;
                                            break;
                                        }
                                    }
                                    if !over_url {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                                    }
                                }
                            }

                            // First pass: paint full-height background rectangles per glyph
                            let rows = &galley.rows;

                            for (row_idx, row) in rows.iter().enumerate() {
                                let row_top = text_pos.y + row.rect.top();
                                // Extend to next row's top, or use current bottom for last row
                                let row_bottom = if row_idx + 1 < rows.len() {
                                    text_pos.y + rows[row_idx + 1].rect.top()
                                } else {
                                    text_pos.y + row.rect.bottom()
                                };

                                for glyph in &row.glyphs {
                                    // Use the glyph's section to get the background color
                                    let section_idx = glyph.section_index as usize;
                                    if section_idx < layout_job.sections.len() {
                                        let bg = layout_job.sections[section_idx].format.background;
                                        if bg != egui::Color32::TRANSPARENT {
                                            let glyph_rect = egui::Rect::from_min_max(
                                                egui::pos2(text_pos.x + glyph.pos.x, row_top),
                                                egui::pos2(text_pos.x + glyph.pos.x + glyph.size.x, row_bottom),
                                            );
                                            painter.rect_filled(glyph_rect, 0.0, bg);
                                        }
                                    }
                                }
                            }

                            // Paint selection highlighting using galley's cursor positioning
                            let selection_color = egui::Color32::from_rgba_unmultiplied(100, 100, 255, 100);
                            if let (Some(sel_start), Some(sel_end)) = (self.selection_start, self.selection_end) {
                                let (start, end) = if sel_start <= sel_end {
                                    (sel_start, sel_end)
                                } else {
                                    (sel_end, sel_start)
                                };

                                if start != end {
                                    // Get cursors for selection bounds
                                    let start_cursor = galley.from_ccursor(egui::text::CCursor::new(start));
                                    let end_cursor = galley.from_ccursor(egui::text::CCursor::new(end));

                                    // Paint selection row by row
                                    let start_row = start_cursor.rcursor.row;
                                    let end_row = end_cursor.rcursor.row;

                                    for row_idx in start_row..=end_row {
                                        if row_idx >= rows.len() {
                                            break;
                                        }
                                        let row = &rows[row_idx];
                                        let row_top = text_pos.y + row.rect.top();
                                        let row_bottom = if row_idx + 1 < rows.len() {
                                            text_pos.y + rows[row_idx + 1].rect.top()
                                        } else {
                                            text_pos.y + row.rect.bottom()
                                        };

                                        // Determine x bounds for this row
                                        let start_x = if row_idx == start_row {
                                            let pos = galley.pos_from_cursor(&start_cursor);
                                            text_pos.x + pos.min.x
                                        } else {
                                            text_pos.x + row.rect.left()
                                        };

                                        let end_x = if row_idx == end_row {
                                            let pos = galley.pos_from_cursor(&end_cursor);
                                            text_pos.x + pos.min.x
                                        } else {
                                            text_pos.x + row.rect.right()
                                        };

                                        if end_x > start_x {
                                            let sel_rect = egui::Rect::from_min_max(
                                                egui::pos2(start_x, row_top),
                                                egui::pos2(end_x, row_bottom),
                                            );
                                            painter.rect_filled(sel_rect, 0.0, selection_color);
                                        }
                                    }
                                }
                            }

                            // Paint the text on top
                            painter.galley(text_pos, galley.clone());

                            // Build cursor_range if we have a selection
                            let cursor_range = if let (Some(sel_start), Some(sel_end)) = (self.selection_start, self.selection_end) {
                                if sel_start != sel_end {
                                    let primary_ccursor = egui::text::CCursor::new(sel_start);
                                    let secondary_ccursor = egui::text::CCursor::new(sel_end);
                                    Some(egui::text_edit::CursorRange {
                                        primary: galley.from_ccursor(primary_ccursor),
                                        secondary: galley.from_ccursor(secondary_ccursor),
                                    })
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            // Wrap in a struct to match TextEdit response interface
                            let response = TextEditOutputWrapper {
                                response: alloc_response,
                                galley,
                                cursor_range,
                                galley_pos: text_pos,
                            };

                            // Store selection in egui memory on every frame when there is one
                            // This ensures we have it captured before any click clears it
                            // Skip storage on secondary click to preserve existing selection
                            // Use world-specific selection IDs to avoid conflicts when switching worlds
                            let selection_id = egui::Id::new(format!("output_selection_{}", self.current_world));
                            let selection_range_id = egui::Id::new(format!("output_selection_range_{}", self.current_world));
                            let selection_raw_id = egui::Id::new(format!("output_selection_raw_{}", self.current_world));
                            let is_secondary_click = response.response.secondary_clicked();
                            if !is_secondary_click {
                            if let Some(cursor_range) = response.cursor_range {
                                if cursor_range.primary != cursor_range.secondary {
                                    let start_char = cursor_range.primary.ccursor.index.min(cursor_range.secondary.ccursor.index);
                                    let end_char = cursor_range.primary.ccursor.index.max(cursor_range.secondary.ccursor.index);
                                    // Convert character indices to byte indices for proper UTF-8 slicing
                                    let galley_text = response.galley.text();
                                    let start_byte = galley_text.char_indices().nth(start_char).map(|(i, _)| i).unwrap_or(galley_text.len());
                                    let end_byte = galley_text.char_indices().nth(end_char).map(|(i, _)| i).unwrap_or(galley_text.len());
                                    let selected = galley_text[start_byte..end_byte].to_string();

                                    // Extract the selected portion from raw lines, preserving ANSI codes
                                    // Helper to extract visible char range from raw text with ANSI
                                    fn extract_raw_selection(raw: &str, vis_start: usize, vis_end: usize) -> String {
                                        let mut result = String::new();
                                        let mut vis_pos = 0;
                                        let mut chars = raw.chars().peekable();

                                        while let Some(c) = chars.next() {
                                            if c == '\x1b' {
                                                // Start of ANSI sequence - include it if we're in selection
                                                let mut seq = String::from(c);
                                                while let Some(&next) = chars.peek() {
                                                    seq.push(chars.next().unwrap());
                                                    if next.is_ascii_alphabetic() {
                                                        break;
                                                    }
                                                }
                                                if vis_pos >= vis_start && vis_pos < vis_end {
                                                    result.push_str(&seq);
                                                }
                                            } else {
                                                // Visible character
                                                if vis_pos >= vis_start && vis_pos < vis_end {
                                                    result.push(c);
                                                }
                                                vis_pos += 1;
                                                if vis_pos >= vis_end {
                                                    break;
                                                }
                                            }
                                        }
                                        result
                                    }

                                    // Build selection from raw lines
                                    let mut raw_selected_parts = Vec::new();
                                    let mut char_pos = 0;
                                    for (i, galley_line) in galley_text.lines().enumerate() {
                                        let line_start = char_pos;
                                        let line_end = char_pos + galley_line.chars().count();

                                        // Check if this line overlaps with selection
                                        if line_end > start_char && line_start < end_char {
                                            if let Some(raw_line) = self.cached_display_lines.get(i) {
                                                // Calculate visible char range within this line
                                                let sel_start_in_line = start_char.saturating_sub(line_start);
                                                let sel_end_in_line = (end_char - line_start).min(galley_line.chars().count());

                                                let raw_part = extract_raw_selection(
                                                    raw_line,
                                                    sel_start_in_line,
                                                    sel_end_in_line
                                                );
                                                if !raw_part.is_empty() {
                                                    raw_selected_parts.push(raw_part);
                                                }
                                            }
                                        }
                                        char_pos = line_end + 1; // +1 for newline
                                    }
                                    let raw_text = raw_selected_parts.join("\n").replace('\x1b', "<esc>");

                                    // Always store selection text, range, and raw lines when we have one
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(selection_id, selected);
                                        d.insert_temp(selection_range_id, (start_char, end_char));
                                        d.insert_temp(selection_raw_id, raw_text);
                                    });
                                }
                            }
                            } // end if !is_secondary_click
                            // Handle clicks - check for URL clicks and clear selection
                            if response.response.clicked() {
                                if let Some(cursor_range) = response.cursor_range {
                                    if cursor_range.primary == cursor_range.secondary {
                                        let click_pos = cursor_range.primary.ccursor.index;

                                        // Check if clicking on a URL (use cached URLs)
                                        let mut url_clicked = false;
                                        for (start, end, url) in &self.cached_urls {
                                            if click_pos >= *start && click_pos <= *end {
                                                // Strip zero-width spaces that were inserted for word breaking
                                                let clean_url = url.replace('\u{200B}', "");
                                                Self::open_url(&clean_url);
                                                url_clicked = true;
                                                break;
                                            }
                                        }

                                        // Clear selection if not clicking a URL
                                        if !url_clicked {
                                            ui.ctx().data_mut(|d| {
                                                d.remove::<String>(selection_id);
                                                d.remove::<(usize, usize)>(selection_range_id);
                                                d.remove::<String>(selection_raw_id);
                                            });
                                        }
                                    }
                                } else {
                                    ui.ctx().data_mut(|d| {
                                        d.remove::<String>(selection_id);
                                        d.remove::<(usize, usize)>(selection_range_id);
                                        d.remove::<String>(selection_raw_id);
                                    });
                                }
                            }

                            // Always draw custom selection highlight when we have a stored selection
                            // This ensures no flicker when context menu opens/closes
                            {
                                if let Some((start, end)) = ui.ctx().data(|d| d.get_temp::<(usize, usize)>(selection_range_id)) {
                                    let galley = &response.galley;
                                    let text_pos = response.galley_pos;

                                    // Get cursor positions for start and end
                                    let start_cursor = galley.from_ccursor(egui::text::CCursor::new(start));
                                    let end_cursor = galley.from_ccursor(egui::text::CCursor::new(end));

                                    // Draw selection rectangles for each row in the selection
                                    let selection_color = egui::Color32::from_rgba_unmultiplied(100, 100, 200, 100);
                                    let painter = ui.painter();

                                    for row_idx in start_cursor.rcursor.row..=end_cursor.rcursor.row {
                                        if let Some(row) = galley.rows.get(row_idx) {
                                            let row_start = if row_idx == start_cursor.rcursor.row {
                                                galley.pos_from_cursor(&start_cursor).min.x
                                            } else {
                                                row.rect.left()
                                            };
                                            let row_end = if row_idx == end_cursor.rcursor.row {
                                                galley.pos_from_cursor(&end_cursor).min.x
                                            } else {
                                                row.rect.right()
                                            };

                                            let rect = egui::Rect::from_min_max(
                                                egui::pos2(text_pos.x + row_start, text_pos.y + row.rect.top()),
                                                egui::pos2(text_pos.x + row_end, text_pos.y + row.rect.bottom()),
                                            );
                                            painter.rect_filled(rect, 0.0, selection_color);
                                        }
                                    }
                                }
                            }

                            // Draw URL underlines and handle hover cursor
                            {
                                let galley = &response.galley;

                                if !self.cached_urls.is_empty() {
                                    let text_pos = response.galley_pos;
                                    let painter = ui.painter();
                                    let link_color = theme.link();
                                    let hover_pos = ui.input(|i| i.pointer.hover_pos());

                                    for (start, end, _url) in &self.cached_urls {
                                        let start_cursor = galley.from_ccursor(egui::text::CCursor::new(*start));
                                        let end_cursor = galley.from_ccursor(egui::text::CCursor::new(*end));

                                        // Draw underline for each row the URL spans
                                        for row_idx in start_cursor.rcursor.row..=end_cursor.rcursor.row {
                                            if let Some(row) = galley.rows.get(row_idx) {
                                                let row_start = if row_idx == start_cursor.rcursor.row {
                                                    galley.pos_from_cursor(&start_cursor).min.x
                                                } else {
                                                    row.rect.left()
                                                };
                                                let row_end = if row_idx == end_cursor.rcursor.row {
                                                    galley.pos_from_cursor(&end_cursor).min.x
                                                } else {
                                                    row.rect.right()
                                                };

                                                // Create rect for this URL segment
                                                let url_rect = egui::Rect::from_min_max(
                                                    egui::pos2(text_pos.x + row_start, text_pos.y + row.rect.top()),
                                                    egui::pos2(text_pos.x + row_end, text_pos.y + row.rect.bottom()),
                                                );

                                                // Check if mouse is hovering over this URL segment
                                                if let Some(pos) = hover_pos {
                                                    if url_rect.contains(pos) {
                                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                                    }
                                                }

                                                // Draw underline at bottom of text
                                                let y = text_pos.y + row.rect.bottom() - 1.0;
                                                painter.line_segment(
                                                    [egui::pos2(text_pos.x + row_start, y), egui::pos2(text_pos.x + row_end, y)],
                                                    egui::Stroke::new(1.0, link_color),
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            // Right-click context menu
                            let plain_text_for_menu = self.cached_plain_text.clone();
                            let debug_request_id = egui::Id::new("debug_text_request");

                            response.response.context_menu(|ui| {
                                // Get stored selection from egui memory
                                let stored_selection: Option<String> = ui.ctx().data(|d| d.get_temp(selection_id));
                                let stored_raw: Option<String> = ui.ctx().data(|d| d.get_temp(selection_raw_id));

                                // Show Copy button if there's stored selected text
                                if let Some(ref selected) = stored_selection {
                                    if ui.button("Copy").clicked() {
                                        ui.ctx().copy_text(selected.clone());
                                        ui.close_menu();
                                    }
                                }
                                if ui.button("Copy All").clicked() {
                                    ui.ctx().copy_text(plain_text_for_menu.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                // Debug option - show raw ANSI codes
                                if let Some(raw_text) = stored_raw {
                                    if ui.button("Debug Selection").clicked() {
                                        // Store in egui memory for retrieval outside closure
                                        ui.ctx().data_mut(|d| d.insert_temp(debug_request_id, raw_text));
                                        ui.close_menu();
                                    }
                                }
                            });
                            // Check if debug was requested
                            let debug_request: Option<String> = ui.ctx().data(|d| {
                                d.get_temp::<String>(debug_request_id)
                            });
                            if let Some(debug_text) = debug_request {
                                self.debug_text = debug_text;
                                self.popup_state = PopupState::DebugText;
                                ui.ctx().data_mut(|d| { d.remove::<String>(debug_request_id); });
                            }
                        });

                    // Track actual scroll position from scroll area state
                    let content_size = scroll_output.content_size.y;
                    let viewport_height = scroll_output.inner_rect.height();
                    let max_offset = (content_size - viewport_height).max(0.0);
                    let current_offset = scroll_output.state.offset.y;

                    // Save max offset for PageUp/PageDown calculations
                    self.scroll_max_offset = max_offset;

                    // Update our tracked offset based on actual scroll position
                    if current_offset >= max_offset - 1.0 {
                        // At or near bottom
                        self.scroll_offset = None;
                    } else if self.scroll_offset.is_some() {
                        // Track actual scroll position (may differ from jump target due to mouse wheel)
                        self.scroll_offset = Some(current_offset.clamp(0.0, max_offset));
                    }
                }

                // Inline search box overlay (upper-right corner, like console F4)
                if self.filter_active {
                    let panel_rect = ui.max_rect();
                    let search_width = 220.0;
                    let search_height = 26.0;
                    let margin = 8.0;
                    let search_rect = egui::Rect::from_min_size(
                        egui::pos2(panel_rect.right() - search_width - margin - 14.0, panel_rect.top() + margin),
                        egui::vec2(search_width, search_height),
                    );

                    let painter = ui.painter();
                    // Background
                    painter.rect_filled(
                        search_rect,
                        egui::Rounding::same(4.0),
                        theme.bg_surface(),
                    );
                    // Border
                    painter.rect_stroke(
                        search_rect,
                        egui::Rounding::same(4.0),
                        egui::Stroke::new(0.5, theme.border_medium()),
                    );

                    // Label
                    let label_pos = egui::pos2(search_rect.left() + 6.0, search_rect.center().y);
                    painter.text(
                        label_pos,
                        egui::Align2::LEFT_CENTER,
                        "Search:",
                        egui::FontId::proportional(11.0),
                        theme.fg_muted(),
                    );

                    // Text input area
                    let input_rect = egui::Rect::from_min_max(
                        egui::pos2(search_rect.left() + 54.0, search_rect.top() + 2.0),
                        egui::pos2(search_rect.right() - 4.0, search_rect.bottom() - 2.0),
                    );
                    let mut child_ui = ui.child_ui(input_rect, egui::Layout::left_to_right(egui::Align::Center));
                    let response = child_ui.add(
                        egui::TextEdit::singleline(&mut self.filter_text)
                            .desired_width(input_rect.width())
                            .frame(false)
                            .font(egui::FontId::monospace(11.0))
                            .text_color(theme.fg())
                    );
                    response.request_focus();
                }
            });

            // Popup windows
            let mut close_popup = false;
            let mut popup_action: Option<(&str, usize)> = None;

            // Worlds popup (combined world selector and connected worlds list) - separate OS window
            if self.popup_state == PopupState::ConnectedWorlds {
                let mut should_close = false;
                let mut selected = self.world_list_selected;
                let mut connect_world: Option<usize> = None;
                let mut edit_world: Option<usize> = None;
                let mut add_world = false;
                let mut toggle_only_connected = false;
                let worlds_clone = self.worlds.clone();
                let current_world = self.current_world;

                let only_connected = self.only_connected_worlds;
                let window_title = if only_connected { "Worlds List" } else { "World Selector" };

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("connected_worlds_window"),
                    egui::ViewportBuilder::default()
                        .with_title(window_title)
                        .with_inner_size([640.0, 352.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("connected_worlds_buttons")
                            .exact_height(44.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 17.0, top: 8.0, bottom: 8.0 }))
                            .show(ctx, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Left side: Connected toggle
                                    ui.label(egui::RichText::new("Connected")
                                        .size(11.0)
                                        .color(theme.fg_secondary())
                                        .family(egui::FontFamily::Monospace));

                                    // Toggle switch (like SSL toggle in world editor)
                                    let toggle_width = 36.0;
                                    let toggle_height = 18.0;
                                    let toggle_rect = ui.allocate_space(egui::vec2(toggle_width, toggle_height)).1;
                                    let toggle_response = ui.interact(toggle_rect, ui.id().with("only_connected_toggle"), egui::Sense::click());

                                    // Draw toggle background
                                    let toggle_bg = if only_connected { theme.accent_dim() } else { theme.bg_deep() };
                                    ui.painter().rect_filled(toggle_rect, egui::Rounding::same(toggle_height / 2.0), toggle_bg);

                                    // Draw toggle knob
                                    let knob_radius = (toggle_height - 4.0) / 2.0;
                                    let knob_x = if only_connected {
                                        toggle_rect.right() - knob_radius - 2.0
                                    } else {
                                        toggle_rect.left() + knob_radius + 2.0
                                    };
                                    let knob_color = if only_connected { theme.bg_deep() } else { theme.fg_muted() };
                                    ui.painter().circle_filled(egui::pos2(knob_x, toggle_rect.center().y), knob_radius, knob_color);

                                    if toggle_response.clicked() {
                                        toggle_only_connected = true;
                                    }

                                    // Spacer to push buttons to right
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                        // Ok button (primary)
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("OK").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                            .fill(theme.accent_dim())
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_close = true;
                                        }

                                        // Connect button
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("CONNECT").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(80.0, 28.0))
                                        ).clicked() {
                                            connect_world = Some(selected);
                                            should_close = true;
                                        }

                                        // Edit button
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("EDIT").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(60.0, 28.0))
                                        ).clicked() {
                                            edit_world = Some(selected);
                                        }

                                        // Add button
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("ADD").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(60.0, 28.0))
                                        ).clicked() {
                                            add_world = true;
                                        }
                                    });
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                // Filter input
                                let filter_rect = ui.allocate_space(egui::vec2(ui.available_width(), 28.0)).1;
                                ui.painter().rect_filled(filter_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                let filter_inner = filter_rect.shrink2(egui::vec2(8.0, 4.0));
                                let mut filter_ui = ui.child_ui(filter_inner, egui::Layout::left_to_right(egui::Align::Center));
                                let filter_edit = TextEdit::singleline(&mut self.connected_worlds_filter)
                                    .frame(false)
                                    .hint_text(egui::RichText::new("Filter worlds...").color(theme.fg_dim()))
                                    .desired_width(filter_inner.width())
                                    .text_color(theme.fg())
                                    .font(egui::FontId::monospace(12.0));
                                filter_ui.add(filter_edit);
                                ui.add_space(12.0);

                                // Table header row
                                let row_height = 24.0;
                                let col_widths = [180.0, 180.0, 60.0, 80.0]; // World, Hostname, Port, User
                                let header_rect = ui.allocate_space(egui::vec2(ui.available_width(), row_height)).1;
                                let header_y = header_rect.center().y;
                                // World header aligned with text after status dot (4 + 14 = 18)
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + 18.0, header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "World",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + col_widths[0], header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "Hostname",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + col_widths[0] + col_widths[1], header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "Port",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + col_widths[0] + col_widths[1] + col_widths[2], header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "User",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());

                                ui.add_space(4.0);
                                ui.add(egui::Separator::default().spacing(0.0));
                                ui.add_space(4.0);

                                // Build filtered list of worlds
                                let filter_lower = self.connected_worlds_filter.to_lowercase();
                                let filtered_worlds: Vec<(usize, &RemoteWorld)> = worlds_clone.iter()
                                    .enumerate()
                                    .filter(|(_, w)| !only_connected || w.connected)
                                    .filter(|(_, w)| {
                                        if filter_lower.is_empty() {
                                            true
                                        } else {
                                            w.name.to_lowercase().contains(&filter_lower) ||
                                            w.settings.hostname.to_lowercase().contains(&filter_lower) ||
                                            w.settings.user.to_lowercase().contains(&filter_lower)
                                        }
                                    })
                                    .collect();

                                let empty_message = if only_connected { "No worlds connected." } else { "No worlds found." };
                                if filtered_worlds.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(empty_message)
                                        .size(12.0)
                                        .color(theme.fg_muted())
                                        .family(egui::FontFamily::Monospace));
                                } else {
                                    let scroll_to = self.popup_scroll_to_selected;
                                    ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                                        for (idx, world) in filtered_worlds.iter() {
                                            let is_current = *idx == current_world;
                                            let is_selected = *idx == selected;

                                            // Full row as a clickable area
                                            let row_rect = ui.allocate_space(egui::vec2(ui.available_width(), row_height)).1;
                                            let response = ui.interact(row_rect, ui.id().with(idx), egui::Sense::click());

                                            // Scroll selected item to center when popup first opens
                                            if is_selected && scroll_to {
                                                ui.scroll_to_rect(row_rect, Some(egui::Align::Center));
                                            }

                                            // Draw selection/hover background for full row
                                            if is_selected {
                                                ui.painter().rect_filled(row_rect, egui::Rounding::same(2.0),
                                                    theme.list_selection_bg());
                                            } else if response.hovered() {
                                                ui.painter().rect_filled(row_rect, egui::Rounding::same(2.0), theme.bg_hover());
                                            }

                                            if response.clicked() {
                                                selected = *idx;
                                            }

                                            // Draw row content
                                            let mut col_x = row_rect.left() + 4.0;
                                            let text_y = row_rect.center().y;

                                            // World column with status dot and current marker
                                            let status_color = if world.connected { theme.success() } else { theme.fg_dim() };
                                            let dot_rect = egui::Rect::from_center_size(
                                                egui::pos2(col_x + 4.0, text_y),
                                                egui::vec2(6.0, 6.0));
                                            ui.painter().circle_filled(dot_rect.center(), 3.0, status_color);
                                            col_x += 14.0;

                                            let current_marker = if is_current { "* " } else { "" };
                                            let name_color = if is_current { theme.accent() } else if is_selected { theme.fg() } else { theme.fg_secondary() };
                                            ui.painter().text(
                                                egui::pos2(col_x, text_y),
                                                egui::Align2::LEFT_CENTER,
                                                format!("{}{}", current_marker, world.name),
                                                egui::FontId::monospace(12.0),
                                                name_color);
                                            col_x = row_rect.left() + col_widths[0];

                                            // Hostname column
                                            ui.painter().text(
                                                egui::pos2(col_x, text_y),
                                                egui::Align2::LEFT_CENTER,
                                                &world.settings.hostname,
                                                egui::FontId::monospace(12.0),
                                                theme.fg_secondary());
                                            col_x += col_widths[1];

                                            // Port column
                                            ui.painter().text(
                                                egui::pos2(col_x, text_y),
                                                egui::Align2::LEFT_CENTER,
                                                &world.settings.port,
                                                egui::FontId::monospace(12.0),
                                                theme.fg_secondary());
                                            col_x += col_widths[2];

                                            // User column
                                            let user_text = if world.settings.user.is_empty() { "—" } else { &world.settings.user };
                                            ui.painter().text(
                                                egui::pos2(col_x, text_y),
                                                egui::Align2::LEFT_CENTER,
                                                user_text,
                                                egui::FontId::monospace(12.0),
                                                theme.fg_secondary());
                                        }
                                    });
                                }
                            });
                    },
                );

                self.world_list_selected = selected;
                self.popup_scroll_to_selected = false;
                if toggle_only_connected {
                    self.only_connected_worlds = !self.only_connected_worlds;
                }
                if let Some(idx) = connect_world {
                    popup_action = Some(("connect", idx));
                }
                if let Some(idx) = edit_world {
                    popup_action = Some(("edit", idx));
                }
                if add_world {
                    popup_action = Some(("add", 0));
                }
                if should_close {
                    close_popup = true;
                }
            }

            // World Editor popup (separate OS window)
            if let PopupState::WorldEditor(world_idx) = self.popup_state {
                let mut should_close = false;
                let mut should_save = false;
                let mut should_connect = false;
                let mut should_delete = false;

                // Copy mutable state for viewport
                let mut edit_name = self.edit_name.clone();
                let mut edit_hostname = self.edit_hostname.clone();
                let mut edit_port = self.edit_port.clone();
                let mut edit_user = self.edit_user.clone();
                let mut edit_password = self.edit_password.clone();
                let mut edit_ssl = self.edit_ssl;
                let mut edit_log_enabled = self.edit_log_enabled;
                let mut edit_encoding = self.edit_encoding;
                let mut edit_auto_login = self.edit_auto_login;
                let mut edit_keep_alive_type = self.edit_keep_alive_type;
                let mut edit_keep_alive_cmd = self.edit_keep_alive_cmd.clone();
                let mut edit_gmcp_packages = self.edit_gmcp_packages.clone();
                let can_delete = self.worlds.len() > 1;

                // Dynamic height based on whether keep-alive cmd is shown
                let popup_height = if edit_keep_alive_type == KeepAliveType::Custom { 531.0 } else { 491.0 };

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("world_editor_window"),
                    egui::ViewportBuilder::default()
                        .with_title("World Editor")
                        .with_inner_size([440.0, popup_height]),
                    |ctx, _class| {
                        // Apply popup styling - remove ALL strokes everywhere
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            // All widget states: NO stroke anywhere
                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = widget_bg;
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.expansion = 0.0;
                            style.visuals.widgets.inactive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.hovered.bg_fill = widget_bg;
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.expansion = 0.0;
                            style.visuals.widgets.hovered.weak_bg_fill = widget_bg;

                            style.visuals.widgets.active.bg_fill = widget_bg;
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.expansion = 0.0;
                            style.visuals.widgets.active.weak_bg_fill = widget_bg;

                            style.visuals.widgets.open.bg_fill = widget_bg;
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.expansion = 0.0;
                            style.visuals.widgets.open.weak_bg_fill = widget_bg;

                            // Selection highlight - no stroke
                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;

                            // Text edit - no cursor stroke
                            style.visuals.extreme_bg_color = widget_bg;
                            style.visuals.text_cursor = egui::Stroke::new(1.0, theme.fg());
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons (right margin reduced to move buttons right)
                        egui::TopBottomPanel::bottom("world_editor_buttons")
                            .exact_height(48.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::new(1.0, theme.border_subtle()))
                                .inner_margin(egui::Margin { left: 16.0, right: 1.0, top: 10.0, bottom: 10.0 }))
                            .show(ctx, |ui| {
                                // Override widget visuals for buttons to match ConnectedWorlds popup
                                {
                                    let style = ui.style_mut();
                                    let widget_rounding = egui::Rounding::same(4.0);
                                    style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                                    style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();
                                    style.visuals.widgets.inactive.rounding = widget_rounding;
                                    style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                                    style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();
                                    style.visuals.widgets.hovered.rounding = widget_rounding;
                                    style.visuals.widgets.active.bg_fill = theme.accent_dim();
                                    style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();
                                    style.visuals.widgets.active.rounding = widget_rounding;
                                }
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Delete button (left side, danger style)
                                    if can_delete
                                        && ui.add(egui::Button::new(
                                            egui::RichText::new("DELETE").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                            .fill(theme.error())
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_delete = true;
                                        }

                                    // Spacer to push remaining buttons to the right
                                    let remaining = ui.available_width() - 240.0; // 3 buttons * 70 + spacing
                                    if remaining > 0.0 {
                                        ui.add_space(remaining);
                                    }

                                    // Cancel button
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Cancel").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_close = true;
                                    }

                                    // Connect button
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Connect").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_save = true;
                                        should_connect = true;
                                    }

                                    // Save button (primary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Save").size(11.0).color(theme.bg_deep()).strong())
                                        .fill(theme.accent_dim())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_save = true;
                                    }
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 20.0, bottom: 6.0 }))
                            .show(ctx, |ui| {
                                // Header
                                ui.label(egui::RichText::new("WORLD EDITOR")
                                    .size(10.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(16.0);

                                // Layout dimensions
                                let label_width = 100.0;
                                let label_spacing = 12.0;
                                let row_height = 28.0;
                                // input_width will be calculated dynamically using available_width()

                                // Helper to draw chevron (down arrow) like mockup SVG
                                let draw_chevron = |painter: &egui::Painter, center: egui::Pos2, color: Color32| {
                                    // Chevron: two lines from top corners to bottom center
                                    // Similar to SVG path "M6 9l6 6 6-6" scaled to fit
                                    let half_width = 5.0;
                                    let half_height = 3.0;
                                    let stroke = egui::Stroke::new(1.5, color);
                                    // Left line: top-left to bottom-center
                                    painter.line_segment(
                                        [egui::pos2(center.x - half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                    // Right line: top-right to bottom-center
                                    painter.line_segment(
                                        [egui::pos2(center.x + half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                };

                                // Helper macro-like closure for form rows
                                let form_row = |ui: &mut egui::Ui, label: &str, add_widget: &mut dyn FnMut(&mut egui::Ui)| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        ui.set_height(row_height);
                                        // Right-aligned label
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(label_width, row_height),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(egui::RichText::new(label.to_uppercase())
                                                    .size(10.0)
                                                    .color(theme.fg_muted()));
                                            }
                                        );
                                        ui.add_space(label_spacing);
                                        add_widget(ui);
                                    });
                                    ui.add_space(6.0);
                                };

                                // Helper to create styled text input - uses available width or fixed
                                // NO BORDERS - just background fill
                                let styled_text_input = |ui: &mut egui::Ui, text: &mut String, fixed_width: Option<f32>, id_salt: &str| {
                                    let width = fixed_width.unwrap_or_else(|| ui.available_width());
                                    let field_id = ui.id().with(id_salt);
                                    let field_rect = ui.allocate_space(egui::vec2(width, row_height)).1;
                                    let _response = ui.interact(field_rect, field_id, egui::Sense::click());

                                    // Draw background only - NO border
                                    ui.painter().rect_filled(field_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    // Inner text edit area (no frame, no background)
                                    let inner_rect = field_rect.shrink2(egui::vec2(8.0, 4.0));
                                    let mut child_ui = ui.child_ui(inner_rect, egui::Layout::left_to_right(egui::Align::Center));
                                    let text_edit = TextEdit::singleline(text)
                                        .frame(false)
                                        .desired_width(inner_rect.width())
                                        .text_color(theme.fg())
                                        .font(egui::FontId::monospace(11.0));
                                    child_ui.add(text_edit);
                                };

                                // Name (full width)
                                form_row(ui, "Name", &mut |ui| {
                                    styled_text_input(ui, &mut edit_name, None, "name_input");
                                });

                                // Hostname (full width)
                                form_row(ui, "Hostname", &mut |ui| {
                                    styled_text_input(ui, &mut edit_hostname, None, "hostname_input");
                                });

                                // Port (fixed width)
                                form_row(ui, "Port", &mut |ui| {
                                    styled_text_input(ui, &mut edit_port, Some(80.0), "port_input");
                                });

                                // User (full width)
                                form_row(ui, "User", &mut |ui| {
                                    styled_text_input(ui, &mut edit_user, None, "user_input");
                                });

                                // Password (full width, not masked)
                                form_row(ui, "Password", &mut |ui| {
                                    styled_text_input(ui, &mut edit_password, None, "password_input");
                                });

                                // Use SSL (toggle)
                                form_row(ui, "Use SSL", &mut |ui| {
                                    // Toggle switch style
                                    let (toggle_bg, toggle_border, knob_pos) = if edit_ssl {
                                        (theme.accent_dim(), theme.accent_dim(), 18.0)
                                    } else {
                                        (theme.bg_deep(), theme.border_medium(), 3.0)
                                    };

                                    let toggle_rect = ui.allocate_space(egui::vec2(36.0, 20.0));
                                    let response = ui.interact(toggle_rect.1, ui.id().with("ssl_toggle"), egui::Sense::click());

                                    if response.clicked() {
                                        edit_ssl = !edit_ssl;
                                    }

                                    // Draw toggle background
                                    ui.painter().rect_filled(
                                        toggle_rect.1,
                                        egui::Rounding::same(10.0),
                                        toggle_bg
                                    );
                                    ui.painter().rect_stroke(
                                        toggle_rect.1,
                                        egui::Rounding::same(10.0),
                                        egui::Stroke::new(1.0, toggle_border)
                                    );

                                    // Draw knob
                                    let knob_color = if edit_ssl { theme.bg_deep() } else { theme.fg_muted() };
                                    let knob_center = egui::pos2(
                                        toggle_rect.1.min.x + knob_pos + 7.0,
                                        toggle_rect.1.center().y
                                    );
                                    ui.painter().circle_filled(knob_center, 7.0, knob_color);
                                });

                                // Logging (toggle)
                                form_row(ui, "Logging", &mut |ui| {
                                    // Toggle switch style
                                    let (toggle_bg, toggle_border, knob_pos) = if edit_log_enabled {
                                        (theme.accent_dim(), theme.accent_dim(), 18.0)
                                    } else {
                                        (theme.bg_deep(), theme.border_medium(), 3.0)
                                    };

                                    let toggle_rect = ui.allocate_space(egui::vec2(36.0, 20.0));
                                    let response = ui.interact(toggle_rect.1, ui.id().with("log_toggle"), egui::Sense::click());

                                    if response.clicked() {
                                        edit_log_enabled = !edit_log_enabled;
                                    }

                                    // Draw toggle background
                                    ui.painter().rect_filled(
                                        toggle_rect.1,
                                        egui::Rounding::same(10.0),
                                        toggle_bg
                                    );
                                    ui.painter().rect_stroke(
                                        toggle_rect.1,
                                        egui::Rounding::same(10.0),
                                        egui::Stroke::new(1.0, toggle_border)
                                    );

                                    // Draw knob
                                    let knob_color = if edit_log_enabled { theme.bg_deep() } else { theme.fg_muted() };
                                    let knob_center = egui::pos2(
                                        toggle_rect.1.min.x + knob_pos + 7.0,
                                        toggle_rect.1.center().y
                                    );
                                    ui.painter().circle_filled(knob_center, 7.0, knob_color);
                                });

                                // Encoding (custom styled dropdown, full width, NO border)
                                form_row(ui, "Encoding", &mut |ui| {
                                    let dropdown_id = ui.id().with("encoding_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    // Background only - NO border
                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        edit_encoding.name(),
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(edit_encoding == Encoding::Utf8,
                                            egui::RichText::new("UTF-8").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_encoding = Encoding::Utf8;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_encoding == Encoding::Latin1,
                                            egui::RichText::new("Latin-1").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_encoding = Encoding::Latin1;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_encoding == Encoding::Fansi,
                                            egui::RichText::new("FANSI").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_encoding = Encoding::Fansi;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });

                                // Auto Login (custom styled dropdown, full width, NO border)
                                form_row(ui, "Auto Login", &mut |ui| {
                                    let dropdown_id = ui.id().with("auto_login_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    // Background only - NO border
                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        edit_auto_login.name(),
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(edit_auto_login == AutoConnectType::Connect,
                                            egui::RichText::new("Connect").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_auto_login = AutoConnectType::Connect;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_auto_login == AutoConnectType::Prompt,
                                            egui::RichText::new("Prompt").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_auto_login = AutoConnectType::Prompt;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_auto_login == AutoConnectType::MooPrompt,
                                            egui::RichText::new("MOO Prompt").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_auto_login = AutoConnectType::MooPrompt;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_auto_login == AutoConnectType::NoLogin,
                                            egui::RichText::new("None").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_auto_login = AutoConnectType::NoLogin;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });

                                // Keep Alive (custom styled dropdown, full width, NO border)
                                form_row(ui, "Keep Alive", &mut |ui| {
                                    let dropdown_id = ui.id().with("keep_alive_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    // Background only - NO border
                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        edit_keep_alive_type.name(),
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(edit_keep_alive_type == KeepAliveType::Nop,
                                            egui::RichText::new("NOP").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_keep_alive_type = KeepAliveType::Nop;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_keep_alive_type == KeepAliveType::Custom,
                                            egui::RichText::new("Custom").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_keep_alive_type = KeepAliveType::Custom;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_keep_alive_type == KeepAliveType::Generic,
                                            egui::RichText::new("Generic").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_keep_alive_type = KeepAliveType::Generic;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });

                                // Only show Keep-Alive CMD when Custom is selected (full width)
                                if edit_keep_alive_type == KeepAliveType::Custom {
                                    form_row(ui, "Keep Alive CMD", &mut |ui| {
                                        styled_text_input(ui, &mut edit_keep_alive_cmd, None, "keep_alive_cmd_input");
                                    });
                                }

                                form_row(ui, "GMCP Packages", &mut |ui| {
                                    styled_text_input(ui, &mut edit_gmcp_packages, None, "gmcp_packages_input");
                                });
                        });
                    },
                );

                // Apply changes back to self
                self.edit_name = edit_name;
                self.edit_hostname = edit_hostname;
                self.edit_port = edit_port;
                self.edit_user = edit_user;
                self.edit_password = edit_password;
                self.edit_ssl = edit_ssl;
                self.edit_log_enabled = edit_log_enabled;
                self.edit_encoding = edit_encoding;
                self.edit_auto_login = edit_auto_login;
                self.edit_keep_alive_type = edit_keep_alive_type;
                self.edit_keep_alive_cmd = edit_keep_alive_cmd;
                self.edit_gmcp_packages = edit_gmcp_packages;

                if should_save {
                    // Update local world settings and send to server
                    if let Some(world) = self.worlds.get_mut(world_idx) {
                        world.name = self.edit_name.clone();
                        world.settings.hostname = self.edit_hostname.clone();
                        world.settings.port = self.edit_port.clone();
                        world.settings.user = self.edit_user.clone();
                        world.settings.password = self.edit_password.clone();
                        world.settings.use_ssl = self.edit_ssl;
                        world.settings.log_enabled = self.edit_log_enabled;
                        world.settings.encoding = self.edit_encoding.name().to_string();
                        world.settings.auto_login = self.edit_auto_login.name().to_string();
                        world.settings.keep_alive_type = self.edit_keep_alive_type.name().to_string();
                        world.settings.keep_alive_cmd = self.edit_keep_alive_cmd.clone();
                        world.settings.gmcp_packages = self.edit_gmcp_packages.clone();
                    }
                    // Send update to server
                    self.update_world_settings(world_idx);
                    if should_connect {
                        popup_action = Some(("connect", world_idx));
                    }
                    close_popup = true;
                } else if should_delete {
                    self.popup_state = PopupState::WorldConfirmDelete(world_idx);
                } else if should_close {
                    close_popup = true;
                }
            }

            // World delete confirmation popup (separate OS window)
            if let PopupState::WorldConfirmDelete(world_idx) = self.popup_state {
                let world_name = self.worlds.get(world_idx)
                    .map(|w| w.name.clone())
                    .unwrap_or_default();
                let mut should_delete = false;
                let mut should_cancel = false;

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("world_confirm_delete_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Confirm Delete")
                        .with_inner_size([320.0, 140.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(20.0)))
                            .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) ||
                               ui.input(|i| i.key_pressed(egui::Key::N)) ||
                               ui.input(|i| i.viewport().close_requested()) {
                                should_cancel = true;
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Y)) {
                                should_delete = true;
                            }

                            // Header
                            ui.label(egui::RichText::new("CONFIRM DELETE")
                                .size(11.0)
                                .color(theme.fg_muted())
                                .strong());
                            ui.add_space(16.0);

                            ui.label(egui::RichText::new(format!("Delete world '{}'?", world_name))
                                .color(theme.fg_secondary()));
                            ui.add_space(20.0);

                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // No button
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("No").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_cancel = true;
                                    }

                                    // Yes button (danger)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Yes").size(11.0).color(theme.error()))
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::new(1.0, theme.error_dim()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_delete = true;
                                    }
                                });
                            });
                        });
                    },
                );

                if should_delete {
                    // Delete the world - send request to server
                    if world_idx < self.worlds.len() && self.worlds.len() > 1 {
                        if let Some(ref ws_tx) = self.ws_tx {
                            let msg = WsMessage::DeleteWorld { world_index: world_idx };
                            let _ = ws_tx.send(msg);
                        }
                        // Local removal will happen when server sends WorldRemoved
                    }
                    // Return to Worlds popup
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.popup_scroll_to_selected = true;
                } else if should_cancel {
                    // Return to Worlds popup
                    self.popup_state = PopupState::ConnectedWorlds;
                    self.popup_scroll_to_selected = true;
                }
            }

            // Setup popup - separate OS window
            if self.popup_state == PopupState::Setup {
                // Save original transparency when popup first opens
                if self.original_transparency.is_none() {
                    self.original_transparency = Some(self.transparency);
                }

                // Copy state for editing in viewport
                let mut more_mode = self.more_mode;
                let mut spell_check = self.spell_check_enabled;
                let mut temp_convert = self.temp_convert_enabled;
                let mut world_switch = self.world_switch_mode;
                let debug_enabled = self.debug_enabled;
                // Note: show_tags is not in setup anymore - controlled by F2 or /tag
                let mut ansi_music = self.ansi_music_enabled;
                let mut tls_proxy = self.tls_proxy_enabled;
                let mut input_height = self.input_height;
                let mut gui_theme = self.theme.clone();
                let mut transparency = self.transparency;
                let mut color_offset = self.color_offset_percent;
                let mut color_offset_dec = false;
                let mut color_offset_inc = false;
                let mut should_close = false;
                let mut should_save = false;
                let mut should_cancel = false;

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("setup_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Settings")
                        .with_inner_size([560.0, 420.0]),
                    |ctx, _class| {
                        // Apply popup styling - remove all default strokes
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = widget_bg;

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = widget_bg;

                            style.visuals.widgets.open.bg_fill = widget_bg;
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = widget_bg;

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_cancel = true;
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("setup_buttons")
                            .exact_height(68.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .stroke(egui::Stroke::NONE))
                            .show(ctx, |ui| {
                                // Use allocate_ui_with_layout for precise vertical positioning
                                let panel_height = ui.available_height();
                                let button_height = 28.0;
                                let bottom_padding = 20.0;
                                let top_padding = panel_height - button_height - bottom_padding;

                                ui.add_space(top_padding);
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);  // Left padding
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_space(18.0);  // Right padding
                                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                        // Save button (primary)
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("SAVE").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                            .fill(theme.accent_dim())
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_save = true;
                                            should_close = true;
                                        }

                                        // Cancel button
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("CANCEL").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_cancel = true;
                                            should_close = true;
                                        }
                                    });
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 20.0, bottom: 16.0 }))
                            .show(ctx, |ui| {
                                // Layout dimensions (matching World Editor)
                                let label_width = 110.0;
                                let label_spacing = 12.0;
                                let row_height = 28.0;

                                // Helper to draw chevron
                                let draw_chevron = |painter: &egui::Painter, center: egui::Pos2, color: Color32| {
                                    let half_width = 5.0;
                                    let half_height = 3.0;
                                    let stroke = egui::Stroke::new(1.5, color);
                                    painter.line_segment(
                                        [egui::pos2(center.x - half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                    painter.line_segment(
                                        [egui::pos2(center.x + half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                };

                                // Helper for form rows
                                let form_row = |ui: &mut egui::Ui, label: &str, add_widget: &mut dyn FnMut(&mut egui::Ui)| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        ui.set_height(row_height);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(label_width, row_height),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(egui::RichText::new(label.to_uppercase())
                                                    .size(10.0)
                                                    .color(theme.fg_muted()));
                                            }
                                        );
                                        ui.add_space(label_spacing);
                                        add_widget(ui);
                                    });
                                    ui.add_space(6.0);
                                };

                                // World Switching (dropdown)
                                form_row(ui, "World Switching", &mut |ui| {
                                    let dropdown_id = ui.id().with("world_switch_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        world_switch.name(),
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(world_switch == WorldSwitchMode::UnseenFirst,
                                            egui::RichText::new("Unseen First").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            world_switch = WorldSwitchMode::UnseenFirst;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(world_switch == WorldSwitchMode::Alphabetical,
                                            egui::RichText::new("Alphabetical").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            world_switch = WorldSwitchMode::Alphabetical;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });

                                // Theme (dropdown)
                                form_row(ui, "Theme", &mut |ui| {
                                    let dropdown_id = ui.id().with("theme_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    let theme_name = gui_theme.name();
                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        theme_name,
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(gui_theme.is_dark(),
                                            egui::RichText::new("Dark").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            gui_theme = GuiTheme::from_name("dark");
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(!gui_theme.is_dark(),
                                            egui::RichText::new("Light").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            gui_theme = GuiTheme::from_name("light");
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });

                                ui.add_space(8.0);

                                // Transparency (right below theme)
                                form_row(ui, "Transparency", &mut |ui| {
                                    let slider_width = ui.available_width();
                                    let slider_height = row_height;
                                    let slider_rect = ui.allocate_space(egui::vec2(slider_width, slider_height)).1;

                                    // Draw track background
                                    let track_rect = egui::Rect::from_center_size(
                                        slider_rect.center(),
                                        egui::vec2(slider_width - 20.0, 4.0)
                                    );
                                    ui.painter().rect_filled(track_rect, egui::Rounding::same(2.0), theme.bg_deep());

                                    // Calculate knob position
                                    let knob_x = track_rect.left() + (transparency - 0.3) / 0.7 * track_rect.width();
                                    let knob_center = egui::pos2(knob_x, slider_rect.center().y);

                                    // Draw filled portion
                                    let filled_rect = egui::Rect::from_min_max(
                                        track_rect.min,
                                        egui::pos2(knob_x, track_rect.max.y)
                                    );
                                    ui.painter().rect_filled(filled_rect, egui::Rounding::same(2.0), theme.accent_dim());

                                    // Draw knob
                                    ui.painter().circle_filled(knob_center, 8.0, theme.accent());

                                    // Handle interaction
                                    let response = ui.interact(slider_rect, ui.id().with("transparency_slider"), egui::Sense::click_and_drag());
                                    if response.dragged() || response.clicked() {
                                        if let Some(pos) = response.interact_pointer_pos() {
                                            let new_value = ((pos.x - track_rect.left()) / track_rect.width() * 0.7 + 0.3)
                                                .clamp(0.3, 1.0);
                                            transparency = new_value;
                                        }
                                    }
                                });

                                // Input Height
                                form_row(ui, "Input Height", &mut |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("-").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_deep())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() && input_height > 1 {
                                        input_height -= 1;
                                    }
                                    ui.add_space(4.0);
                                    // Number display in a styled box
                                    let num_rect = ui.allocate_space(egui::vec2(40.0, row_height)).1;
                                    ui.painter().rect_filled(num_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                    ui.painter().text(
                                        num_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{}", input_height),
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );
                                    ui.add_space(4.0);
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("+").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_deep())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() && input_height < 15 {
                                        input_height += 1;
                                    }
                                });

                                // Color Offset (0 = off, 5-100 = percentage)
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                    ui.set_height(row_height);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(label_width, row_height),
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(egui::RichText::new("COLOR OFFSET")
                                                .size(10.0)
                                                .color(theme.fg_muted()));
                                        }
                                    );
                                    ui.add_space(label_spacing);
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("-").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_deep())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() {
                                        color_offset_dec = true;
                                    }
                                    ui.add_space(4.0);
                                    // Number display in a styled box
                                    let num_rect = ui.allocate_space(egui::vec2(56.0, row_height)).1;
                                    ui.painter().rect_filled(num_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                    let label = if color_offset == 0 { "OFF".to_string() } else { format!("{}%", color_offset) };
                                    ui.painter().text(
                                        num_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        label,
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );
                                    ui.add_space(4.0);
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("+").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_deep())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() {
                                        color_offset_inc = true;
                                    }
                                });
                                ui.add_space(6.0);

                                ui.add_space(8.0);

                                // Toggle switches in two columns
                                let switch_width = 44.0;
                                let switch_height = 22.0;
                                // Use same label_width as form_row for alignment
                                let col_total_width = label_width + label_spacing + switch_width + 16.0;

                                // Helper for toggle column with right-aligned label (same as form_row)
                                let toggle_col = |ui: &mut egui::Ui, label: &str, id: &str, enabled: &mut bool| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(label_width, row_height),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(egui::RichText::new(label)
                                                    .size(10.0)
                                                    .color(theme.fg_muted()));
                                            }
                                        );
                                        ui.add_space(label_spacing);
                                        let switch_rect = ui.allocate_space(egui::vec2(switch_width, switch_height)).1;
                                        let response = ui.interact(switch_rect, ui.id().with(id), egui::Sense::click());
                                        let track_color = if *enabled { theme.accent_dim() } else { theme.bg_deep() };
                                        ui.painter().rect_filled(switch_rect, egui::Rounding::same(11.0), track_color);
                                        let knob_x = if *enabled { switch_rect.right() - 11.0 } else { switch_rect.left() + 11.0 };
                                        let knob_color = if *enabled { theme.accent() } else { theme.fg_muted() };
                                        ui.painter().circle_filled(egui::pos2(knob_x, switch_rect.center().y), 7.0, knob_color);
                                        if response.clicked() { *enabled = !*enabled; }
                                    });
                                };

                                // Row 1: More Mode | Spell Check
                                ui.horizontal(|ui| {
                                    ui.set_height(row_height);
                                    ui.allocate_ui(egui::vec2(col_total_width, row_height), |ui| {
                                        toggle_col(ui, "MORE MODE", "more_mode_toggle", &mut more_mode);
                                    });
                                    ui.allocate_ui(egui::vec2(col_total_width, row_height), |ui| {
                                        toggle_col(ui, "SPELL CHECK", "spell_check_toggle", &mut spell_check);
                                    });
                                });
                                ui.add_space(6.0);

                                // Row 2: ANSI Music | Temp Convert
                                ui.horizontal(|ui| {
                                    ui.set_height(row_height);
                                    ui.allocate_ui(egui::vec2(col_total_width, row_height), |ui| {
                                        toggle_col(ui, "ANSI MUSIC", "ansi_music_toggle", &mut ansi_music);
                                    });
                                    ui.allocate_ui(egui::vec2(col_total_width, row_height), |ui| {
                                        toggle_col(ui, "TEMP CONVERT", "temp_convert_toggle", &mut temp_convert);
                                    });
                                });
                                ui.add_space(6.0);

                                // Row 3: TLS Proxy
                                ui.horizontal(|ui| {
                                    ui.set_height(row_height);
                                    ui.allocate_ui(egui::vec2(col_total_width, row_height), |ui| {
                                        toggle_col(ui, "TLS PROXY", "tls_proxy_toggle", &mut tls_proxy);
                                    });
                                });
                        });
                    },
                );

                // Handle color offset button clicks (flags set inside viewport closure)
                if color_offset_dec && color_offset > 0 {
                    color_offset = color_offset.saturating_sub(5);
                }
                if color_offset_inc && color_offset < 100 {
                    color_offset = (color_offset + 5).min(100);
                }

                // Apply changes back (live preview for transparency)
                self.more_mode = more_mode;
                self.spell_check_enabled = spell_check;
                self.temp_convert_enabled = temp_convert;
                self.world_switch_mode = world_switch;
                self.debug_enabled = debug_enabled;
                // Note: show_tags is not in setup anymore - controlled by F2 or /tag
                self.ansi_music_enabled = ansi_music;
                self.tls_proxy_enabled = tls_proxy;
                self.input_height = input_height;
                self.theme = gui_theme;
                self.transparency = transparency;
                self.color_offset_percent = color_offset;

                if should_save {
                    self.update_global_settings();
                    self.original_transparency = None;
                }
                if should_cancel {
                    // Revert transparency to original value
                    if let Some(orig) = self.original_transparency.take() {
                        self.transparency = orig;
                    }
                }
                if should_close {
                    self.original_transparency = None;
                    close_popup = true;
                }
            }

            // Web popup (matches console /web) - separate OS window
            if self.popup_state == PopupState::Web {
                let mut should_close = false;
                let mut should_save = false;

                // Copy mutable state for viewport
                let mut web_secure = self.web_secure;
                let mut http_enabled = self.http_enabled;
                let mut http_port = self.http_port;
                let mut ws_enabled = self.ws_enabled;
                let mut ws_port = self.ws_port;
                let mut ws_allow_list = self.ws_allow_list.clone();
                let mut ws_cert_file = self.ws_cert_file.clone();
                let mut ws_key_file = self.ws_key_file.clone();

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("web_settings_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Web Settings")
                        .with_inner_size([380.0, 400.0])
                        .with_resizable(false),
                    |ctx, _class| {
                        // Apply popup styling - remove all default strokes
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = widget_bg;

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = widget_bg;

                            style.visuals.widgets.open.bg_fill = widget_bg;
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = widget_bg;

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons - use allocate_space for precise positioning
                        egui::TopBottomPanel::bottom("web_settings_buttons")
                            .exact_height(65.0)
                            .frame(egui::Frame::none().fill(theme.bg_elevated()))
                            .show(ctx, |ui| {
                                ui.vertical(|ui| {
                                    // Top padding (space between content and buttons)
                                    ui.add_space(15.0);

                                    // Buttons row
                                    ui.horizontal(|ui| {
                                        ui.add_space(16.0); // left padding

                                        // Spacer to push buttons right
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(18.0); // right padding

                                            // Save button (primary) - rightmost
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("Save").size(11.0).color(theme.bg_deep()).strong())
                                                .fill(theme.accent_dim())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(70.0, 28.0))
                                            ).clicked() {
                                                should_save = true;
                                            }

                                            ui.add_space(8.0);

                                            // Cancel button
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("Cancel").size(11.0).color(theme.fg_secondary()))
                                                .fill(theme.bg_hover())
                                                .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(70.0, 28.0))
                                            ).clicked() {
                                                should_close = true;
                                            }
                                        });
                                    });

                                    // Bottom padding (space between buttons and window edge)
                                    ui.add_space(20.0);
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin { left: 16.0, right: 0.0, top: 16.0, bottom: 16.0 }))
                            .show(ctx, |ui| {
                                // Header
                                ui.label(egui::RichText::new("WEB SETTINGS")
                                    .size(11.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(16.0);

                                egui::Grid::new("web_grid")
                                    .num_columns(2)
                                    .spacing([16.0, 10.0])
                                    .show(ui, |ui| {
                                        // Protocol selection
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new("Protocol").size(12.0).color(theme.fg_secondary()));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("Secure").size(11.0)
                                                    .color(if web_secure { theme.bg_deep() } else { theme.fg_muted() }))
                                                .fill(if web_secure { theme.accent_dim() } else { theme.bg_hover() })
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(70.0, 24.0))
                                            ).clicked() {
                                                web_secure = true;
                                            }
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("Non-Secure").size(11.0)
                                                    .color(if !web_secure { theme.bg_deep() } else { theme.fg_muted() }))
                                                .fill(if !web_secure { theme.accent_dim() } else { theme.bg_hover() })
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(80.0, 24.0))
                                            ).clicked() {
                                                web_secure = false;
                                            }
                                        });
                                        ui.end_row();

                                        // HTTP/HTTPS enabled
                                        let http_label = if web_secure { "HTTPS enabled" } else { "HTTP enabled" };
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new(http_label).size(12.0).color(theme.fg_secondary()));
                                        });
                                        let http_text = if http_enabled { "ON" } else { "OFF" };
                                        let http_color = if http_enabled { theme.accent() } else { theme.fg_muted() };
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new(http_text).size(11.0).color(http_color))
                                            .fill(if http_enabled { theme.accent_dim() } else { theme.bg_hover() })
                                            .stroke(egui::Stroke::new(1.0, if http_enabled { theme.accent_dim() } else { theme.border_medium() }))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(50.0, 24.0))
                                        ).clicked() {
                                            http_enabled = !http_enabled;
                                        }
                                        ui.end_row();

                                        // HTTP/HTTPS port
                                        let http_port_label = if web_secure { "HTTPS port" } else { "HTTP port" };
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new(http_port_label).size(12.0).color(theme.fg_secondary()));
                                        });
                                        let mut http_port_str = http_port.to_string();
                                        let field_width = ui.available_width();
                                        if ui.add(egui::TextEdit::singleline(&mut http_port_str)
                                            .text_color(theme.fg())
                                            .desired_width(field_width)
                                            .margin(egui::vec2(8.0, 6.0))).changed() {
                                            if let Ok(port) = http_port_str.parse::<u16>() {
                                                http_port = port;
                                            }
                                        }
                                        ui.end_row();

                                        // WS/WSS enabled
                                        let ws_label = if web_secure { "WSS enabled" } else { "WS enabled" };
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new(ws_label).size(12.0).color(theme.fg_secondary()));
                                        });
                                        let ws_text = if ws_enabled { "ON" } else { "OFF" };
                                        let ws_color = if ws_enabled { theme.accent() } else { theme.fg_muted() };
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new(ws_text).size(11.0).color(ws_color))
                                            .fill(if ws_enabled { theme.accent_dim() } else { theme.bg_hover() })
                                            .stroke(egui::Stroke::new(1.0, if ws_enabled { theme.accent_dim() } else { theme.border_medium() }))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(50.0, 24.0))
                                        ).clicked() {
                                            ws_enabled = !ws_enabled;
                                        }
                                        ui.end_row();

                                        // WS/WSS port
                                        let ws_port_label = if web_secure { "WSS port" } else { "WS port" };
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new(ws_port_label).size(12.0).color(theme.fg_secondary()));
                                        });
                                        let mut ws_port_str = ws_port.to_string();
                                        let field_width = ui.available_width();
                                        if ui.add(egui::TextEdit::singleline(&mut ws_port_str)
                                            .text_color(theme.fg())
                                            .desired_width(field_width)
                                            .margin(egui::vec2(8.0, 6.0))).changed() {
                                            if let Ok(port) = ws_port_str.parse::<u16>() {
                                                ws_port = port;
                                            }
                                        }
                                        ui.end_row();

                                        // Allow list
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new("Allow List").size(12.0).color(theme.fg_secondary()));
                                        });
                                        let field_width = ui.available_width();
                                        ui.add(egui::TextEdit::singleline(&mut ws_allow_list)
                                            .text_color(theme.fg())
                                            .hint_text("localhost, 192.168.*")
                                            .desired_width(field_width)
                                            .margin(egui::vec2(8.0, 6.0)));
                                        ui.end_row();

                                        // TLS cert/key files (only show when Secure is selected)
                                        if web_secure {
                                            // TLS Cert File
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new("TLS Cert File").size(12.0).color(theme.fg_secondary()));
                                            });
                                            let field_width = ui.available_width();
                                            ui.add(egui::TextEdit::singleline(&mut ws_cert_file)
                                                .text_color(theme.fg())
                                                .hint_text("/path/to/cert.pem")
                                                .desired_width(field_width)
                                                .margin(egui::vec2(8.0, 6.0)));
                                            ui.end_row();

                                            // TLS Key File
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new("TLS Key File").size(12.0).color(theme.fg_secondary()));
                                            });
                                            let field_width = ui.available_width();
                                            ui.add(egui::TextEdit::singleline(&mut ws_key_file)
                                                .text_color(theme.fg())
                                                .hint_text("/path/to/key.pem")
                                                .desired_width(field_width)
                                                .margin(egui::vec2(8.0, 6.0)));
                                            ui.end_row();
                                        }
                                    });
                        });
                    },
                );

                // Apply changes back to self
                self.web_secure = web_secure;
                self.http_enabled = http_enabled;
                self.http_port = http_port;
                self.ws_enabled = ws_enabled;
                self.ws_port = ws_port;
                self.ws_allow_list = ws_allow_list;
                self.ws_cert_file = ws_cert_file;
                self.ws_key_file = ws_key_file;

                if should_save {
                    self.update_global_settings();
                    close_popup = true;
                } else if should_close {
                    close_popup = true;
                }
            }

            // Font popup - separate OS window
            if self.popup_state == PopupState::Font {
                // Common monospace font families
                const FONT_FAMILIES: &[(&str, &str)] = &[
                    ("", "System Default"),
                    ("Monospace", "Monospace"),
                    ("DejaVu Sans Mono", "DejaVu Sans Mono"),
                    ("Liberation Mono", "Liberation Mono"),
                    ("Ubuntu Mono", "Ubuntu Mono"),
                    ("Fira Code", "Fira Code"),
                    ("Source Code Pro", "Source Code Pro"),
                    ("JetBrains Mono", "JetBrains Mono"),
                    ("Hack", "Hack"),
                    ("Inconsolata", "Inconsolata"),
                    ("Courier New", "Courier New"),
                    ("Consolas", "Consolas"),
                ];

                let mut should_close = false;
                let mut should_save = false;

                // Copy mutable state for viewport
                let mut edit_font_name = self.edit_font_name.clone();
                let mut edit_font_size = self.edit_font_size.clone();
                let mut edit_font_scale = self.edit_font_scale.clone();
                let mut edit_font_y_offset = self.edit_font_y_offset.clone();
                let mut edit_font_baseline_offset = self.edit_font_baseline_offset.clone();
                let scroll_to_selected = self.popup_scroll_to_selected;

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("font_settings_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Font Settings")
                        .with_inner_size([380.0, 480.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.fg_muted());
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = widget_bg;

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = widget_bg;

                            style.visuals.widgets.open.bg_fill = widget_bg;
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = widget_bg;

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("font_settings_buttons")
                            .exact_height(48.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 17.0, top: 10.0, bottom: 10.0 }))
                            .show(ctx, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Spacer to push buttons to the right
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                        // OK button (primary) - rightmost
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("OK").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                            .fill(theme.accent_dim())
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_save = true;
                                        }

                                        // Cancel button
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("CANCEL").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_close = true;
                                        }
                                    });
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                // Header
                                ui.label(egui::RichText::new("FONT SETTINGS")
                                    .size(11.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(16.0);

                                // Font family label and list
                                ui.label(egui::RichText::new("Font family").size(12.0).color(theme.fg_secondary()));
                                ui.add_space(4.0);

                                egui::Frame::none()
                                    .fill(theme.bg_deep())
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::same(4.0))
                                    .show(ui, |ui| {
                                        let scroll_to = scroll_to_selected;
                                        egui::ScrollArea::vertical()
                                            .max_height(288.0)
                                            .show(ui, |ui| {
                                                ui.set_min_width(ui.available_width());
                                                for (value, label) in FONT_FAMILIES {
                                                    let is_selected = *value == edit_font_name;
                                                    let resp = ui.selectable_label(is_selected,
                                                        egui::RichText::new(*label).size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace));
                                                    if is_selected && scroll_to {
                                                        resp.scroll_to_me(Some(egui::Align::Center));
                                                    }
                                                    if resp.clicked() {
                                                        edit_font_name = value.to_string();
                                                    }
                                                }
                                            });
                                    });

                                ui.add_space(12.0);

                                // Font size label and controls
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Font size").size(12.0).color(theme.fg_secondary()));
                                    ui.add_space(8.0);
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("-").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() {
                                        if let Ok(size) = edit_font_size.parse::<f32>() {
                                            let new_size = (size - 1.0).max(8.0);
                                            edit_font_size = format!("{:.1}", new_size);
                                        }
                                    }
                                    ui.add(egui::TextEdit::singleline(&mut edit_font_size)
                                        .desired_width(50.0)
                                        .margin(egui::vec2(8.0, 6.0)));
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("+").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(28.0, 24.0))
                                    ).clicked() {
                                        if let Ok(size) = edit_font_size.parse::<f32>() {
                                            let new_size = (size + 1.0).min(48.0);
                                            edit_font_size = format!("{:.1}", new_size);
                                        }
                                    }
                                });

                                ui.add_space(16.0);

                                // Font tweaks section
                                ui.label(egui::RichText::new("Font tweaks").size(12.0).color(theme.fg_secondary()));
                                ui.add_space(4.0);

                                // Helper for tweak rows with -/+ buttons
                                let tweak_row = |ui: &mut egui::Ui, label: &str, value: &mut String, step: f32, min: f32, max: f32| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(label).size(11.0).color(theme.fg_muted())
                                            .family(egui::FontFamily::Monospace));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("+").size(11.0).color(theme.fg_secondary()))
                                                .fill(theme.bg_hover())
                                                .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(24.0, 22.0))
                                            ).clicked() {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    *value = format!("{:.2}", (v + step).min(max));
                                                }
                                            }
                                            ui.add(egui::TextEdit::singleline(value)
                                                .desired_width(50.0)
                                                .margin(egui::vec2(6.0, 4.0)));
                                            if ui.add(egui::Button::new(
                                                egui::RichText::new("-").size(11.0).color(theme.fg_secondary()))
                                                .fill(theme.bg_hover())
                                                .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                                .rounding(egui::Rounding::same(4.0))
                                                .min_size(egui::vec2(24.0, 22.0))
                                            ).clicked() {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    *value = format!("{:.2}", (v - step).max(min));
                                                }
                                            }
                                        });
                                    });
                                };

                                tweak_row(ui, "Scale",    &mut edit_font_scale,           0.05, 0.50, 2.00);
                                tweak_row(ui, "Y Offset", &mut edit_font_y_offset,         0.01, -0.50, 0.50);
                                tweak_row(ui, "Baseline", &mut edit_font_baseline_offset,   0.01, -0.50, 0.50);
                        });
                    },
                );

                // Apply changes back to self
                self.edit_font_name = edit_font_name;
                self.edit_font_size = edit_font_size;
                self.edit_font_scale = edit_font_scale;
                self.edit_font_y_offset = edit_font_y_offset;
                self.edit_font_baseline_offset = edit_font_baseline_offset;
                self.popup_scroll_to_selected = false;

                if should_save {
                    // Parse and apply font settings
                    self.font_name = self.edit_font_name.clone();
                    if let Ok(size) = self.edit_font_size.parse::<f32>() {
                        self.font_size = size.clamp(8.0, 48.0);
                    }
                    if let Ok(v) = self.edit_font_scale.parse::<f32>() {
                        self.font_scale = v.clamp(0.50, 2.00);
                    }
                    if let Ok(v) = self.edit_font_y_offset.parse::<f32>() {
                        self.font_y_offset = v.clamp(-0.50, 0.50);
                    }
                    if let Ok(v) = self.edit_font_baseline_offset.parse::<f32>() {
                        self.font_baseline_offset = v.clamp(-0.50, 0.50);
                    }
                    // Send updated settings to server and save locally
                    self.update_global_settings();
                    self.save_remote_settings();
                    close_popup = true;
                } else if should_close {
                    close_popup = true;
                }
            }

            // Help popup - separate OS window
            if self.popup_state == PopupState::Help {
                let mut should_close = false;
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("help_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Help - Clay MUD Client")
                        .with_inner_size([450.0, 400.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        // Check for Escape key or window close
                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for button
                        egui::TopBottomPanel::bottom("help_buttons")
                            .exact_height(44.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 1.0, top: 8.0, bottom: 8.0 }))
                            .show(ctx, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("OK").size(11.0).color(theme.bg_deep()).strong())
                                        .fill(theme.accent_dim())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_close = true;
                                    }
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                // Title
                                ui.label(egui::RichText::new("CLAY MUD CLIENT")
                                    .size(11.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(12.0);

                                egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        // World Switching section
                                        ui.label(egui::RichText::new("World Switching")
                                            .size(12.0)
                                            .color(theme.accent())
                                            .strong());
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new("  Up/Down         Cycle through active worlds")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Shift+Up/Down   Cycle through all worlds")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.add_space(12.0);

                                        // Output Navigation section
                                        ui.label(egui::RichText::new("Output Navigation")
                                            .size(12.0)
                                            .color(theme.accent())
                                            .strong());
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new("  PageUp/Down     Scroll through output history")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Tab             Release one screenful (when paused)")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Alt+J           Jump to end, release all pending")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.add_space(12.0);

                                        // Input section
                                        ui.label(egui::RichText::new("Input")
                                            .size(12.0)
                                            .color(theme.accent())
                                            .strong());
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new("  Enter           Send command")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Ctrl+P/N        Previous/Next command history")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Ctrl+U          Clear input line")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Ctrl+W          Delete word before cursor")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Ctrl+Q          Spell check suggestions")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.add_space(12.0);

                                        // Display section
                                        ui.label(egui::RichText::new("Display")
                                            .size(12.0)
                                            .color(theme.accent())
                                            .strong());
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new("  F2              Toggle MUD tag display")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  F4              Open filter popup")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.add_space(12.0);

                                        // Options Menu section
                                        ui.label(egui::RichText::new("Options Menu")
                                            .size(12.0)
                                            .color(theme.accent())
                                            .strong());
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new("  World List      View and select worlds")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  World Editor    Edit world connection settings")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Settings        Global settings")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Font            Change font family and size")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Connect         Connect to current world")
                                            .size(11.0).color(theme.fg_secondary()));
                                        ui.label(egui::RichText::new("  Disconnect      Disconnect from current world")
                                            .size(11.0).color(theme.fg_secondary()));
                                    });
                            });
                    },
                );
                if should_close {
                    close_popup = true;
                }
            }

            // Menu popup - separate OS window
            if self.popup_state == PopupState::Menu {
                let mut should_close = false;
                let mut selected_command: Option<String> = None;
                let menu_items = [
                    ("Help", "/help"),
                    ("Settings", "/setup"),
                    ("Web Settings", "/web"),
                    ("Actions", "/actions"),
                    ("World Selector", "/worlds"),
                    ("Connected Worlds", "/connections"),
                ];

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("menu_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Menu")
                        .with_inner_size([220.0, 250.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                        });

                        // Handle keyboard
                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }
                        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                            if self.menu_selected > 0 {
                                self.menu_selected -= 1;
                            } else {
                                self.menu_selected = menu_items.len() - 1;
                            }
                        }
                        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                            if self.menu_selected < menu_items.len() - 1 {
                                self.menu_selected += 1;
                            } else {
                                self.menu_selected = 0;
                            }
                        }
                        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                            selected_command = Some(menu_items[self.menu_selected].1.to_string());
                            should_close = true;
                        }

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(12.0)))
                            .show(ctx, |ui| {
                                ui.vertical(|ui| {
                                    for (i, (label, _cmd)) in menu_items.iter().enumerate() {
                                        let is_selected = i == self.menu_selected;
                                        let bg_color = if is_selected { theme.accent_dim() } else { Color32::TRANSPARENT };
                                        let text_color = if is_selected { theme.bg_deep() } else { theme.fg() };

                                        let response = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new(*label)
                                                    .size(12.0)
                                                    .color(text_color)
                                                    .strong()
                                            )
                                            .fill(bg_color)
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(190.0, 28.0))
                                        );

                                        if response.clicked() {
                                            selected_command = Some(menu_items[i].1.to_string());
                                            should_close = true;
                                        }
                                    }

                                    ui.add_space(12.0);
                                    ui.label(egui::RichText::new("↑↓ select, Enter open")
                                        .size(10.0)
                                        .color(theme.fg_secondary()));
                                });
                            });
                    },
                );

                if let Some(cmd) = selected_command {
                    // Execute the command
                    let parsed = super::parse_command(&cmd);
                    match parsed {
                        super::Command::Help => self.popup_state = PopupState::Help,
                        super::Command::Version => {
                            let ts = current_timestamp_secs();
                            if self.current_world < self.worlds.len() {
                                let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                self.worlds[self.current_world].output_lines.push(
                                    TimestampedLine { text: super::get_version_string(), ts, gagged: false, from_server: false, seq, highlight_color: None }
                                );
                            }
                        }
                        super::Command::Setup => self.popup_state = PopupState::Setup,
                        super::Command::Web => self.popup_state = PopupState::Web,
                        super::Command::Actions { .. } => {
                            self.open_actions_list_unified();
                        }
                        super::Command::WorldSelector => {
                            self.popup_state = PopupState::ConnectedWorlds;
                            self.world_list_selected = self.current_world;
                            self.only_connected_worlds = false;
                            self.popup_scroll_to_selected = true;
                        }
                        super::Command::WorldsList => {
                            // Output connected worlds list as text (no window)
                            let worlds_info: Vec<super::util::WorldListInfo> = self.worlds.iter().enumerate().map(|(idx, world)| {
                                super::util::WorldListInfo {
                                    name: world.name.clone(),
                                    connected: world.connected,
                                    is_current: idx == self.current_world,
                                    is_ssl: world.settings.use_ssl,
                                    is_proxy: world.is_proxy,
                                    unseen_lines: world.unseen_lines,
                                    last_send_secs: world.last_send_secs,
                                    last_recv_secs: world.last_recv_secs,
                                    last_nop_secs: world.last_nop_secs,
                                    next_nop_secs: None,
                                    buffer_size: world.output_lines.len(),
                                }
                            }).collect();
                            let output = super::util::format_worlds_list(&worlds_info);
                            let ts = super::current_timestamp_secs();
                            if self.current_world < self.worlds.len() {
                                for line in output.lines() {
                                    let seq = self.worlds[self.current_world].output_lines.len() as u64;
                                    self.worlds[self.current_world].output_lines.push(TimestampedLine {
                                        text: line.to_string(),
                                        ts,
                                        gagged: false,
                                        from_server: false,
                                        seq,
                                        highlight_color: None,
                                    });
                                }
                            }
                        }
                        _ => close_popup = true,
                    }
                } else if should_close {
                    close_popup = true;
                }
            }

            // Debug Text popup - separate OS window
            if self.popup_state == PopupState::DebugText {
                let mut should_close = false;
                let debug_text_clone = self.debug_text.clone();

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("debug_text_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Debug - Raw ANSI Codes")
                        .with_inner_size([600.0, 250.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("debug_buttons")
                            .exact_height(44.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 1.0, top: 8.0, bottom: 8.0 }))
                            .show(ctx, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Close button (primary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Close").size(11.0).color(theme.bg_deep()).strong())
                                        .fill(theme.accent_dim())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_close = true;
                                    }

                                    // Copy button (secondary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("Copy").size(11.0).color(theme.fg_secondary()))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        ui.ctx().copy_text(debug_text_clone.clone());
                                    }
                                });
                            });

                        // Central panel for content
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                // Header
                                ui.label(egui::RichText::new("RAW ANSI CODES")
                                    .size(11.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("ESC character shown as <esc>")
                                    .size(10.0)
                                    .color(theme.fg_muted()));
                                ui.add_space(12.0);

                                // Content area with background
                                let display_text = if debug_text_clone.is_empty() {
                                    "(No text captured)".to_string()
                                } else {
                                    debug_text_clone.clone()
                                };

                                egui::Frame::none()
                                    .fill(theme.bg_deep())
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::same(12.0))
                                    .show(ui, |ui| {
                                        egui::ScrollArea::both()
                                            .auto_shrink([false; 2])
                                            .show(ui, |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(&display_text)
                                                            .monospace()
                                                            .size(11.0)
                                                            .color(theme.fg_secondary())
                                                    ).wrap(true)
                                                );
                                            });
                                    });
                            });
                    },
                );
                if should_close {
                    close_popup = true;
                }
            }

            // Actions List popup (separate OS window)
            if self.popup_state == PopupState::ActionsList {
                let mut should_close = false;
                let mut new_popup_state: Option<PopupState> = None;
                let mut actions_selected = self.actions_selected;
                let mut actions_list_filter = self.actions_list_filter.clone();
                let actions_clone = self.actions.clone();
                let scroll_to_selected = self.popup_scroll_to_selected;

                // State for opening editor
                let mut open_editor_idx: Option<usize> = None;
                let mut add_new_action = false;

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("actions_list_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Actions - Clay MUD Client")
                        .with_inner_size([580.0, 400.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("actions_buttons")
                            .exact_height(48.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 17.0, top: 10.0, bottom: 10.0 }))
                            .show(ctx, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Delete button (danger) - left aligned
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("DELETE").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                        .fill(theme.error())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() && !actions_clone.is_empty() {
                                        new_popup_state = Some(PopupState::ActionConfirmDelete);
                                    }

                                    // Spacer to push remaining buttons to the right
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                        // OK button (primary)
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("OK").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                            .fill(theme.accent_dim())
                                            .stroke(egui::Stroke::NONE)
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            should_close = true;
                                        }

                                        // Edit button (secondary)
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("EDIT").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() && !actions_clone.is_empty() {
                                            open_editor_idx = Some(actions_selected);
                                        }

                                        // Add button (secondary)
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("ADD").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                            .fill(theme.bg_hover())
                                            .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                            .rounding(egui::Rounding::same(4.0))
                                            .min_size(egui::vec2(70.0, 28.0))
                                        ).clicked() {
                                            add_new_action = true;
                                        }
                                    });
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                // Filter input
                                let filter_rect = ui.allocate_space(egui::vec2(ui.available_width(), 28.0)).1;
                                ui.painter().rect_filled(filter_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                let filter_inner = filter_rect.shrink2(egui::vec2(8.0, 4.0));
                                let mut filter_ui = ui.child_ui(filter_inner, egui::Layout::left_to_right(egui::Align::Center));
                                let filter_edit = TextEdit::singleline(&mut actions_list_filter)
                                    .frame(false)
                                    .hint_text(egui::RichText::new("Filter actions...").color(theme.fg_dim()))
                                    .desired_width(filter_inner.width())
                                    .text_color(theme.fg())
                                    .font(egui::FontId::monospace(12.0));
                                filter_ui.add(filter_edit);
                                ui.add_space(12.0);

                                // Table header row
                                let row_height = 24.0;
                                let col_widths = [140.0, 100.0, 220.0]; // Name, World, Pattern
                                let header_rect = ui.allocate_space(egui::vec2(ui.available_width(), row_height)).1;
                                let header_y = header_rect.center().y;
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + 4.0, header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "Name",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + col_widths[0], header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "World",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());
                                ui.painter().text(
                                    egui::pos2(header_rect.left() + col_widths[0] + col_widths[1], header_y),
                                    egui::Align2::LEFT_CENTER,
                                    "Pattern",
                                    egui::FontId::monospace(11.0),
                                    theme.fg_muted());

                                ui.add_space(4.0);
                                ui.add(egui::Separator::default().spacing(0.0));
                                ui.add_space(4.0);

                                // Build filtered list of actions
                                let filter_lower = actions_list_filter.to_lowercase();
                                let filtered_actions: Vec<(usize, &Action)> = actions_clone.iter()
                                    .enumerate()
                                    .filter(|(_, a)| {
                                        if filter_lower.is_empty() {
                                            true
                                        } else {
                                            a.name.to_lowercase().contains(&filter_lower) ||
                                            a.world.to_lowercase().contains(&filter_lower) ||
                                            a.pattern.to_lowercase().contains(&filter_lower)
                                        }
                                    })
                                    .collect();

                                if filtered_actions.is_empty() {
                                    ui.add_space(8.0);
                                    let msg = if actions_clone.is_empty() { "No actions defined." } else { "No actions found." };
                                    ui.label(egui::RichText::new(msg)
                                        .size(12.0)
                                        .color(theme.fg_muted())
                                        .family(egui::FontFamily::Monospace));
                                } else {
                                    let scroll_to = scroll_to_selected;
                                    ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                                        for (idx, action) in filtered_actions.iter() {
                                            let is_selected = *idx == actions_selected;

                                            // Full row as a clickable area
                                            let row_rect = ui.allocate_space(egui::vec2(ui.available_width(), row_height)).1;
                                            let response = ui.interact(row_rect, ui.id().with(idx), egui::Sense::click());

                                            // Scroll selected item to center when popup first opens
                                            if is_selected && scroll_to {
                                                ui.scroll_to_rect(row_rect, Some(egui::Align::Center));
                                            }

                                            // Draw selection/hover background for full row
                                            if is_selected {
                                                ui.painter().rect_filled(row_rect, egui::Rounding::same(2.0),
                                                    theme.list_selection_bg());
                                            } else if response.hovered() {
                                                ui.painter().rect_filled(row_rect, egui::Rounding::same(2.0), theme.bg_hover());
                                            }

                                            if response.clicked() {
                                                actions_selected = *idx;
                                            }

                                            // Draw row content
                                            let col_x = row_rect.left() + 4.0;
                                            let text_y = row_rect.center().y;

                                            // Name column
                                            let name_color = if is_selected { theme.fg() } else { theme.fg_secondary() };
                                            ui.painter().text(
                                                egui::pos2(col_x, text_y),
                                                egui::Align2::LEFT_CENTER,
                                                &action.name,
                                                egui::FontId::monospace(12.0),
                                                name_color);

                                            // World column
                                            let world_display = if action.world.is_empty() { "(all)" } else { &action.world };
                                            ui.painter().text(
                                                egui::pos2(row_rect.left() + col_widths[0], text_y),
                                                egui::Align2::LEFT_CENTER,
                                                world_display,
                                                egui::FontId::monospace(12.0),
                                                theme.fg_muted());

                                            // Pattern column
                                            let pattern_display = if action.pattern.is_empty() { "(manual)" } else { &action.pattern };
                                            ui.painter().text(
                                                egui::pos2(row_rect.left() + col_widths[0] + col_widths[1], text_y),
                                                egui::Align2::LEFT_CENTER,
                                                pattern_display,
                                                egui::FontId::monospace(12.0),
                                                theme.fg_muted());
                                        }
                                    });
                                }
                            });
                    },
                );

                // Apply changes back to self
                self.actions_selected = actions_selected;
                self.actions_list_filter = actions_list_filter;
                self.popup_scroll_to_selected = false;

                if let Some(state) = new_popup_state {
                    self.popup_state = state;
                } else if let Some(idx) = open_editor_idx {
                    // Load selected action into editor
                    if let Some(action) = self.actions.get(idx) {
                        self.edit_action_name = action.name.clone();
                        self.edit_action_world = action.world.clone();
                        self.edit_action_match_type = action.match_type;
                        self.edit_action_pattern = action.pattern.clone();
                        self.edit_action_command = action.command.clone();
                        self.edit_action_enabled = action.enabled;
                        self.edit_action_startup = action.startup;
                        self.action_error = None;
                        self.popup_state = PopupState::ActionEditor(idx);
                    }
                } else if add_new_action {
                    // Create new action and open editor
                    self.edit_action_name = String::new();
                    self.edit_action_world = String::new();
                    self.edit_action_match_type = MatchType::Regexp;
                    self.edit_action_pattern = String::new();
                    self.edit_action_command = String::new();
                    self.edit_action_enabled = true;
                    self.edit_action_startup = false;
                    self.action_error = None;
                    self.popup_state = PopupState::ActionEditor(usize::MAX); // MAX = new action
                } else if should_close {
                    close_popup = true;
                }
            }

            // Actions Editor popup (separate OS window)
            if let PopupState::ActionEditor(edit_idx) = self.popup_state.clone() {
                let title = if edit_idx == usize::MAX { "New Action - Clay MUD Client" } else { "Edit Action - Clay MUD Client" };
                let mut should_close = false;
                let mut should_save = false;

                // Copy mutable state for viewport
                let mut edit_action_name = self.edit_action_name.clone();
                let mut edit_action_world = self.edit_action_world.clone();
                let mut edit_action_match_type = self.edit_action_match_type;
                let mut edit_action_pattern = self.edit_action_pattern.clone();
                let mut edit_action_command = self.edit_action_command.clone();
                let mut edit_action_enabled = self.edit_action_enabled;
                let mut edit_action_startup = self.edit_action_startup;
                let mut action_error = self.action_error.clone();
                let actions_clone = self.actions.clone();

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("actions_editor_window"),
                    egui::ViewportBuilder::default()
                        .with_title(title)
                        .with_inner_size([450.0, 340.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("action_editor_buttons")
                            .exact_height(48.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 17.0, top: 10.0, bottom: 10.0 }))
                            .show(ctx, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // Cancel button (secondary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("CANCEL").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_close = true;
                                    }

                                    // Save button (primary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("SAVE").size(11.0).color(theme.bg_deep()).strong().family(egui::FontFamily::Monospace))
                                        .fill(theme.accent_dim())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        // Validate
                                        let name = edit_action_name.trim();
                                        if name.is_empty() {
                                            action_error = Some("Name is required".to_string());
                                        } else {
                                            // Check for duplicates (excluding current if editing)
                                            let mut duplicate = false;
                                            for (i, a) in actions_clone.iter().enumerate() {
                                                if (edit_idx == usize::MAX || i != edit_idx) &&
                                                   a.name.eq_ignore_ascii_case(name) {
                                                    action_error = Some(format!("Action '{}' already exists", name));
                                                    duplicate = true;
                                                    break;
                                                }
                                            }
                                            if !duplicate {
                                                should_save = true;
                                            }
                                        }
                                    }
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 20.0, bottom: 16.0 }))
                            .show(ctx, |ui| {
                                // Layout dimensions (matching World Editor)
                                let label_width = 90.0;
                                let label_spacing = 12.0;
                                let row_height = 28.0;

                                // Helper to draw chevron (down arrow) like World Editor
                                let draw_chevron = |painter: &egui::Painter, center: egui::Pos2, color: Color32| {
                                    let half_width = 5.0;
                                    let half_height = 3.0;
                                    let stroke = egui::Stroke::new(1.5, color);
                                    painter.line_segment(
                                        [egui::pos2(center.x - half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                    painter.line_segment(
                                        [egui::pos2(center.x + half_width, center.y - half_height),
                                         egui::pos2(center.x, center.y + half_height)],
                                        stroke
                                    );
                                };

                                // Helper for form rows with right-aligned uppercase labels
                                let form_row = |ui: &mut egui::Ui, label: &str, add_widget: &mut dyn FnMut(&mut egui::Ui)| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        ui.set_height(row_height);
                                        // Right-aligned label
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(label_width, row_height),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(egui::RichText::new(label.to_uppercase())
                                                    .size(10.0)
                                                    .color(theme.fg_muted()));
                                            }
                                        );
                                        ui.add_space(label_spacing);
                                        add_widget(ui);
                                    });
                                    ui.add_space(6.0);
                                };

                                // Helper for styled text input (no border, just background)
                                let styled_text_input = |ui: &mut egui::Ui, text: &mut String, hint: Option<&str>, id_salt: &str| {
                                    let width = ui.available_width();
                                    let field_rect = ui.allocate_space(egui::vec2(width, row_height)).1;
                                    ui.painter().rect_filled(field_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                    let inner_rect = field_rect.shrink2(egui::vec2(8.0, 4.0));
                                    let mut child_ui = ui.child_ui(inner_rect, egui::Layout::left_to_right(egui::Align::Center));
                                    let mut text_edit = TextEdit::singleline(text)
                                        .frame(false)
                                        .desired_width(inner_rect.width())
                                        .text_color(theme.fg())
                                        .font(egui::FontId::monospace(11.0));
                                    if let Some(h) = hint {
                                        text_edit = text_edit.hint_text(egui::RichText::new(h).color(theme.fg_dim()));
                                    }
                                    child_ui.add(text_edit);
                                    let _ = id_salt; // Used for uniqueness if needed
                                };

                                // Name
                                form_row(ui, "Name", &mut |ui| {
                                    styled_text_input(ui, &mut edit_action_name, None, "action_name");
                                });

                                // World
                                form_row(ui, "World", &mut |ui| {
                                    styled_text_input(ui, &mut edit_action_world, Some("(empty = all worlds)"), "action_world");
                                });

                                // Match Type (custom styled dropdown, full width, NO border - exactly like World Editor)
                                form_row(ui, "Match Type", &mut |ui| {
                                    let dropdown_id = ui.id().with("match_type_dropdown");
                                    let _is_open = ui.memory(|mem| mem.is_popup_open(dropdown_id));
                                    let dropdown_width = ui.available_width();

                                    let button_rect = ui.allocate_space(egui::vec2(dropdown_width, row_height)).1;
                                    let response = ui.interact(button_rect, dropdown_id.with("button"), egui::Sense::click());

                                    // Background only - NO border
                                    ui.painter().rect_filled(button_rect, egui::Rounding::same(4.0), theme.bg_deep());

                                    let match_type_text = match edit_action_match_type {
                                        MatchType::Regexp => "Regexp",
                                        MatchType::Wildcard => "Wildcard",
                                    };
                                    ui.painter().text(
                                        egui::pos2(button_rect.min.x + 12.0, button_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        match_type_text,
                                        egui::FontId::monospace(11.0),
                                        theme.fg()
                                    );

                                    draw_chevron(ui.painter(), egui::pos2(button_rect.max.x - 16.0, button_rect.center().y), theme.fg_muted());

                                    if response.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(dropdown_id));
                                    }

                                    egui::popup_below_widget(ui, dropdown_id, &response, |ui| {
                                        ui.set_min_width(dropdown_width);
                                        ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.fg());
                                        if ui.selectable_label(edit_action_match_type == MatchType::Regexp,
                                            egui::RichText::new("Regexp").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_action_match_type = MatchType::Regexp;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                        if ui.selectable_label(edit_action_match_type == MatchType::Wildcard,
                                            egui::RichText::new("Wildcard").size(11.0).color(theme.fg()).family(egui::FontFamily::Monospace)).clicked() {
                                            edit_action_match_type = MatchType::Wildcard;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    });
                                });
                                ui.add_space(6.0);

                                // Pattern
                                let pattern_hint = match edit_action_match_type {
                                    MatchType::Regexp => "(regex, empty = manual only)",
                                    MatchType::Wildcard => "(wildcard: * ?, empty = manual only)",
                                };
                                form_row(ui, "Pattern", &mut |ui| {
                                    styled_text_input(ui, &mut edit_action_pattern, Some(pattern_hint), "action_pattern");
                                });

                                // Enabled toggle
                                form_row(ui, "Enabled", &mut |ui| {
                                    let text = if edit_action_enabled { "Yes" } else { "No" };
                                    let btn = egui::Button::new(
                                        egui::RichText::new(text)
                                            .size(11.0)
                                            .color(theme.fg())
                                            .family(egui::FontFamily::Monospace)
                                    )
                                    .fill(theme.bg_deep())
                                    .stroke(egui::Stroke::NONE)
                                    .min_size(egui::vec2(60.0, 24.0));
                                    if ui.add(btn).clicked() {
                                        edit_action_enabled = !edit_action_enabled;
                                    }
                                });

                                // Startup toggle
                                form_row(ui, "Startup", &mut |ui| {
                                    let text = if edit_action_startup { "Yes" } else { "No" };
                                    let btn = egui::Button::new(
                                        egui::RichText::new(text)
                                            .size(11.0)
                                            .color(theme.fg())
                                            .family(egui::FontFamily::Monospace)
                                    )
                                    .fill(theme.bg_deep())
                                    .stroke(egui::Stroke::NONE)
                                    .min_size(egui::vec2(60.0, 24.0));
                                    if ui.add(btn).clicked() {
                                        edit_action_startup = !edit_action_startup;
                                    }
                                });

                                ui.add_space(4.0);

                                // Command label (right-aligned like other labels)
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(label_width, row_height),
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(egui::RichText::new("COMMAND")
                                                .size(10.0)
                                                .color(theme.fg_muted()));
                                        }
                                    );
                                });
                                ui.add_space(4.0);

                                // Command area with background (full width)
                                let cmd_rect = ui.allocate_space(egui::vec2(ui.available_width(), 100.0)).1;
                                ui.painter().rect_filled(cmd_rect, egui::Rounding::same(4.0), theme.bg_deep());
                                let cmd_inner = cmd_rect.shrink2(egui::vec2(8.0, 6.0));
                                let mut cmd_ui = ui.child_ui(cmd_inner, egui::Layout::left_to_right(egui::Align::TOP));
                                cmd_ui.add(egui::TextEdit::multiline(&mut edit_action_command)
                                    .frame(false)
                                    .hint_text(egui::RichText::new("Commands (semicolon-separated)").color(theme.fg_dim()))
                                    .desired_width(cmd_inner.width())
                                    .desired_rows(3)
                                    .text_color(theme.fg())
                                    .font(egui::FontId::monospace(11.0)));

                                // Error message
                                if let Some(ref err) = action_error {
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(err)
                                        .size(11.0)
                                        .color(theme.error()));
                                }
                            });
                    },
                );

                // Apply changes back to self
                self.edit_action_name = edit_action_name;
                self.edit_action_world = edit_action_world;
                self.edit_action_match_type = edit_action_match_type;
                self.edit_action_pattern = edit_action_pattern;
                self.edit_action_command = edit_action_command;
                self.edit_action_enabled = edit_action_enabled;
                self.edit_action_startup = edit_action_startup;
                self.action_error = action_error;

                if should_save {
                    let new_action = Action {
                        name: self.edit_action_name.trim().to_string(),
                        world: self.edit_action_world.trim().to_string(),
                        match_type: self.edit_action_match_type,
                        pattern: self.edit_action_pattern.clone(),
                        command: self.edit_action_command.clone(),
                        owner: None,
                        enabled: self.edit_action_enabled,
                        startup: self.edit_action_startup,
                    };
                    if edit_idx == usize::MAX {
                        // New action
                        self.actions.push(new_action);
                        self.actions_selected = self.actions.len() - 1;
                    } else {
                        // Update existing
                        self.actions[edit_idx] = new_action;
                    }
                    // Send updated actions to server
                    self.update_actions();
                    self.popup_state = PopupState::ActionsList;
                    self.popup_scroll_to_selected = true;
                } else if should_close {
                    self.popup_state = PopupState::ActionsList;
                    self.popup_scroll_to_selected = true;
                }
            }

            // Action delete confirmation popup (separate OS window)
            if self.popup_state == PopupState::ActionConfirmDelete {
                let mut should_close = false;
                let mut should_delete = false;
                let action_name = self.actions.get(self.actions_selected)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "(unknown)".to_string());

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("action_confirm_delete_window"),
                    egui::ViewportBuilder::default()
                        .with_title("Confirm Delete - Clay MUD Client")
                        .with_inner_size([340.0, 140.0]),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.noninteractive.rounding = widget_rounding;
                            style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.inactive.rounding = widget_rounding;
                            style.visuals.widgets.inactive.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.hovered.rounding = widget_rounding;
                            style.visuals.widgets.hovered.weak_bg_fill = theme.bg_hover();

                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.active.rounding = widget_rounding;
                            style.visuals.widgets.active.weak_bg_fill = theme.accent_dim();

                            style.visuals.widgets.open.bg_fill = theme.bg_hover();
                            style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                            style.visuals.widgets.open.rounding = widget_rounding;
                            style.visuals.widgets.open.weak_bg_fill = theme.bg_hover();

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close = true;
                        }

                        // Bottom panel for buttons
                        egui::TopBottomPanel::bottom("action_confirm_delete_buttons")
                            .exact_height(44.0)
                            .frame(egui::Frame::none()
                                .fill(theme.bg_surface())
                                .stroke(egui::Stroke::NONE)
                                .inner_margin(egui::Margin { left: 16.0, right: 17.0, top: 8.0, bottom: 8.0 }))
                            .show(ctx, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

                                    // No button (secondary)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("NO").size(11.0).color(theme.fg_secondary()).family(egui::FontFamily::Monospace))
                                        .fill(theme.bg_hover())
                                        .stroke(egui::Stroke::new(1.0, theme.border_medium()))
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_close = true;
                                    }

                                    // Yes button (danger)
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("YES").size(11.0).color(Color32::WHITE).strong().family(egui::FontFamily::Monospace))
                                        .fill(theme.error_dim())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(4.0))
                                        .min_size(egui::vec2(70.0, 28.0))
                                    ).clicked() {
                                        should_delete = true;
                                    }
                                });
                            });

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin::same(16.0)))
                            .show(ctx, |ui| {
                                ui.label(egui::RichText::new("CONFIRM DELETE")
                                    .size(11.0)
                                    .color(theme.fg_muted())
                                    .strong());
                                ui.add_space(16.0);

                                ui.label(egui::RichText::new(format!("Delete action '{}'?", action_name))
                                    .size(12.0)
                                    .color(theme.fg()));
                            });
                    },
                );

                if should_delete {
                    if self.actions_selected < self.actions.len() {
                        self.actions.remove(self.actions_selected);
                        if self.actions_selected >= self.actions.len() && !self.actions.is_empty() {
                            self.actions_selected = self.actions.len() - 1;
                        }
                        // Send updated actions to server
                        self.update_actions();
                    }
                    self.popup_state = PopupState::ActionsList;
                    self.popup_scroll_to_selected = true;
                } else if should_close {
                    self.popup_state = PopupState::ActionsList;
                    self.popup_scroll_to_selected = true;
                }
            }

            // Unified popup rendering
            if let Some(ref mut popup_state) = self.unified_popup {
                let popup_title = popup_state.definition.title.clone();
                let popup_id_str = popup_state.definition.id.0.to_string();
                let mut should_close_unified = false;
                let mut clicked_button: Option<crate::popup::ButtonId> = None;

                // Calculate popup size based on layout
                let min_width = popup_state.definition.layout.min_width as f32;
                let label_width = popup_state.definition.layout.label_width as f32;
                let scroll_to_selected = self.popup_scroll_to_selected;

                // Calculate height from content
                let row_height = 28.0_f32;
                let row_spacing = 8.0_f32;
                let mut content_height = 0.0_f32;
                for field in &popup_state.definition.fields {
                    if !field.visible { continue; }
                    let h = match &field.kind {
                        crate::popup::FieldKind::MultilineText { visible_lines, .. } => {
                            *visible_lines as f32 * 18.0 + 8.0
                        }
                        crate::popup::FieldKind::List { visible_height, headers, .. } => {
                            let header = if headers.is_some() { row_height + row_spacing } else { 0.0 };
                            header + *visible_height as f32 * (row_height + 2.0)
                        }
                        crate::popup::FieldKind::ScrollableContent { visible_height, .. } => {
                            *visible_height as f32 * 18.0
                        }
                        _ => row_height,
                    };
                    content_height += h + row_spacing;
                }
                // Add space for buttons and margins
                let buttons_height = if popup_state.definition.buttons.is_empty() { 0.0 } else { 48.0 };
                let margins = 16.0 + 15.0; // top + bottom inner margin
                let popup_height = (content_height + buttons_height + margins + 15.0).max(200.0);

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of(format!("unified_popup_{}", popup_id_str)),
                    egui::ViewportBuilder::default()
                        .with_title(format!("{} - Clay MUD Client", popup_title))
                        .with_inner_size([min_width.max(400.0), popup_height])
                        .with_resizable(true),
                    |ctx, _class| {
                        // Apply popup styling
                        ctx.style_mut(|style| {
                            style.visuals.window_fill = theme.bg_elevated();
                            style.visuals.panel_fill = theme.bg_elevated();
                            style.visuals.window_stroke = egui::Stroke::NONE;
                            style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                            let widget_bg = theme.bg_deep();
                            let widget_rounding = egui::Rounding::same(4.0);

                            style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                            style.visuals.widgets.inactive.bg_fill = theme.bg_hover();
                            style.visuals.widgets.hovered.bg_fill = theme.bg_hover();
                            style.visuals.widgets.active.bg_fill = theme.accent_dim();
                            style.visuals.widgets.open.bg_fill = theme.bg_hover();

                            for w in [
                                &mut style.visuals.widgets.noninteractive,
                                &mut style.visuals.widgets.inactive,
                                &mut style.visuals.widgets.hovered,
                                &mut style.visuals.widgets.active,
                                &mut style.visuals.widgets.open,
                            ] {
                                w.bg_stroke = egui::Stroke::NONE;
                                w.fg_stroke = egui::Stroke::NONE;
                                w.rounding = widget_rounding;
                            }

                            style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 211, 238, 38);
                            style.visuals.selection.stroke = egui::Stroke::NONE;
                            style.visuals.extreme_bg_color = widget_bg;
                        });

                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
                           ctx.input(|i| i.viewport().close_requested()) {
                            should_close_unified = true;
                        }

                        egui::CentralPanel::default()
                            .frame(egui::Frame::none()
                                .fill(theme.bg_elevated())
                                .inner_margin(egui::Margin { left: 16.0, right: 16.0, top: 16.0, bottom: 15.0 }))
                            .show(ctx, |ui| {
                                // Create GUI popup theme from current theme
                                let gui_theme = crate::popup::gui_renderer::GuiPopupTheme::from_colors(
                                    theme.bg_elevated(),
                                    theme.bg_surface(),
                                    theme.bg_deep(),
                                    theme.bg_hover(),
                                    theme.fg(),
                                    theme.fg_muted(),
                                    theme.fg_muted(),
                                    theme.accent(),
                                    theme.accent_dim(),
                                    theme.fg_dim(),  // border color
                                    theme.error(),
                                );

                                let actions = crate::popup::gui_renderer::render_popup_content_with_scroll(
                                    ui,
                                    popup_state,
                                    &gui_theme,
                                    label_width.max(80.0),
                                    scroll_to_selected,
                                );

                                // Store clicked button for processing outside viewport
                                if let Some(btn_id) = actions.clicked_button {
                                    clicked_button = Some(btn_id);
                                }

                                // Apply other actions to state
                                crate::popup::gui_renderer::apply_actions(popup_state, actions);
                            });
                    },
                );

                self.popup_scroll_to_selected = false;

                // Handle button clicks outside viewport closure
                if let Some(btn_id) = clicked_button {
                    // Check if this is a close/cancel/ok button
                    let popup_id = self.unified_popup.as_ref().map(|p| p.definition.id.0);
                    match popup_id {
                        Some("connections") => {
                            // Connections popup just closes
                            should_close_unified = true;
                        }
                        Some("actions_list") => {
                            use crate::popup::definitions::actions::*;
                            if btn_id == ACTIONS_BTN_CANCEL {
                                should_close_unified = true;
                            } else if btn_id == ACTIONS_BTN_ADD {
                                // Open action editor for new action
                                let settings = ActionSettings::default();
                                let def = create_action_editor_popup(&settings, true);
                                self.unified_popup = Some(crate::popup::PopupState::new(def));
                            } else if btn_id == ACTIONS_BTN_EDIT {
                                // Get selected action index from list
                                if let Some(ps) = &self.unified_popup {
                                    if let Some(field) = ps.field(ACTIONS_FIELD_LIST) {
                                        if let crate::popup::FieldKind::List { selected_index, items, .. } = &field.kind {
                                            if let Some(item) = items.get(*selected_index) {
                                                if let Ok(idx) = item.id.parse::<usize>() {
                                                    if let Some(action) = self.actions.get(idx) {
                                                        let settings = ActionSettings {
                                                            name: action.name.clone(),
                                                            world: action.world.clone(),
                                                            match_type: action.match_type.as_str().to_string(),
                                                            pattern: action.pattern.clone(),
                                                            command: action.command.clone(),
                                                            enabled: action.enabled,
                                                            startup: action.startup,
                                                        };
                                                        self.actions_selected = idx;
                                                        let def = create_action_editor_popup(&settings, false);
                                                        self.unified_popup = Some(crate::popup::PopupState::new(def));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if btn_id == ACTIONS_BTN_DELETE {
                                // Get selected action and open confirm dialog
                                if let Some(ps) = &self.unified_popup {
                                    if let Some(field) = ps.field(ACTIONS_FIELD_LIST) {
                                        if let crate::popup::FieldKind::List { selected_index, items, .. } = &field.kind {
                                            if let Some(item) = items.get(*selected_index) {
                                                if let Ok(idx) = item.id.parse::<usize>() {
                                                    if let Some(action) = self.actions.get(idx) {
                                                        let def = crate::popup::definitions::confirm::create_delete_action_dialog(&action.name);
                                                        self.actions_selected = idx;
                                                        self.unified_popup = Some(crate::popup::PopupState::new(def));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some("action_editor") => {
                            use crate::popup::definitions::actions::*;
                            if btn_id == EDITOR_BTN_CANCEL {
                                // Return to actions list
                                self.open_actions_list_unified();
                            } else if btn_id == EDITOR_BTN_DELETE {
                                // Delete action - open confirm dialog
                                if self.actions_selected < self.actions.len() {
                                    let name = self.actions[self.actions_selected].name.clone();
                                    let def = crate::popup::definitions::confirm::create_delete_action_dialog(&name);
                                    self.unified_popup = Some(crate::popup::PopupState::new(def));
                                }
                            } else if btn_id == EDITOR_BTN_SAVE {
                                // Save action and return to list
                                if let Some(ps) = &self.unified_popup {
                                    let editing_idx = ps.field(EDITOR_FIELD_NAME)
                                        .and_then(|_| ps.definition.id.0.strip_prefix("action_editor"))
                                        .and_then(|s| s.parse::<usize>().ok());

                                    let name = ps.field(EDITOR_FIELD_NAME)
                                        .and_then(|f| f.kind.get_text())
                                        .unwrap_or("")
                                        .to_string();
                                    let world = ps.field(EDITOR_FIELD_WORLD)
                                        .and_then(|f| f.kind.get_text())
                                        .unwrap_or("")
                                        .to_string();
                                    let pattern = ps.field(EDITOR_FIELD_PATTERN)
                                        .and_then(|f| f.kind.get_text())
                                        .unwrap_or("")
                                        .to_string();
                                    let command = ps.field(EDITOR_FIELD_COMMAND)
                                        .and_then(|f| f.kind.get_text())
                                        .unwrap_or("")
                                        .to_string();
                                    let match_type_idx = ps.field(EDITOR_FIELD_MATCH_TYPE)
                                        .and_then(|f| if let crate::popup::FieldKind::Select { selected_index, .. } = &f.kind {
                                            Some(*selected_index)
                                        } else { None })
                                        .unwrap_or(0);
                                    let enabled = ps.field(EDITOR_FIELD_ENABLED)
                                        .and_then(|f| if let crate::popup::FieldKind::Toggle { value } = &f.kind {
                                            Some(*value)
                                        } else { None })
                                        .unwrap_or(true);
                                    let startup = ps.field(EDITOR_FIELD_STARTUP)
                                        .and_then(|f| if let crate::popup::FieldKind::Toggle { value } = &f.kind {
                                            Some(*value)
                                        } else { None })
                                        .unwrap_or(false);

                                    let action = Action {
                                        name,
                                        world,
                                        pattern,
                                        command,
                                        match_type: if match_type_idx == 0 { super::MatchType::Regexp } else { super::MatchType::Wildcard },
                                        enabled,
                                        startup,
                                        owner: None,
                                    };

                                    if let Some(idx) = editing_idx {
                                        if idx < self.actions.len() {
                                            self.actions[idx] = action;
                                        }
                                    } else {
                                        self.actions.push(action);
                                    }

                                    self.update_actions();
                                }
                                self.open_actions_list_unified();
                            }
                        }
                        Some("delete_action") => {
                            use crate::popup::definitions::confirm::*;
                            if btn_id == CONFIRM_BTN_YES {
                                // Delete the action
                                if self.actions_selected < self.actions.len() {
                                    self.actions.remove(self.actions_selected);
                                    if self.actions_selected >= self.actions.len() && !self.actions.is_empty() {
                                        self.actions_selected = self.actions.len() - 1;
                                    }
                                    self.update_actions();
                                }
                                self.open_actions_list_unified();
                            } else if btn_id == CONFIRM_BTN_NO {
                                self.open_actions_list_unified();
                            }
                        }
                        Some("delete_world") => {
                            use crate::popup::definitions::confirm::*;
                            if btn_id == CONFIRM_BTN_YES {
                                // Get world index from custom_data and send delete request to server
                                if let Some(ps) = &self.unified_popup {
                                    if let Some(world_index_str) = ps.definition.custom_data.get("world_index") {
                                        if let Ok(world_index) = world_index_str.parse::<usize>() {
                                            if let Some(ref ws_tx) = self.ws_tx {
                                                let msg = WsMessage::DeleteWorld { world_index };
                                                let _ = ws_tx.send(msg);
                                            }
                                        }
                                    }
                                }
                                self.open_world_selector_unified();
                            } else if btn_id == CONFIRM_BTN_NO {
                                self.open_world_selector_unified();
                            }
                        }
                        _ => {
                            // Generic close on any button for unknown popups
                            should_close_unified = true;
                        }
                    }
                }

                if should_close_unified {
                    self.unified_popup = None;
                }
            }

            // Handle popup actions
            if let Some((action, idx)) = popup_action {
                match action {
                    "connect" => self.connect_world(idx),
                    "edit" => self.open_world_editor(idx),
                    "switch" => {
                        self.current_world = idx;
                        self.selection_start = None; self.selection_end = None;
                        self.switch_world(idx);
                    }
                    "delete" => {
                        if self.worlds.len() > 1 && idx < self.worlds.len() {
                            self.popup_state = PopupState::WorldConfirmDelete(idx);
                        }
                    }
                    "add" => {
                        // Create world locally and open editor immediately
                        let new_name = format!("World {}", self.worlds.len() + 1);
                        let new_world = RemoteWorld {
                            name: new_name.clone(),
                            connected: false,
                            was_connected: false,
                            is_proxy: false,
                            output_lines: Vec::new(),
                            prompt: String::new(),
                            settings: RemoteWorldSettings::default(),
                            unseen_lines: 0,
                            pending_count: 0,
                            last_send_secs: None,
                            last_recv_secs: None,
                            last_nop_secs: None,
                            partial_line: String::new(),
                            showing_splash: true,
                            gmcp_user_enabled: false,
                        };
                        self.worlds.push(new_world);
                        let new_idx = self.worlds.len() - 1;
                        if let Some(ref tx) = self.ws_tx {
                            let _ = tx.send(WsMessage::CreateWorld { name: new_name });
                        }
                        self.open_world_editor(new_idx);
                    }
                    _ => {}
                }
            }

            if close_popup {
                self.popup_state = PopupState::None;
            }
        }
    }
}

/// Load the application icon from embedded PNG data
fn load_app_icon() -> Option<egui::IconData> {
    // Embed the icon PNG at compile time
    const ICON_PNG: &[u8] = include_bytes!("../clay_icon.png");

    // Decode the PNG using the image crate
    let img = image::load_from_memory(ICON_PNG).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

/// Run the remote GUI client
pub fn run(addr: &str, runtime: tokio::runtime::Handle) -> io::Result<()> {
    // Check for display server availability (Linux only - Windows/macOS always have a display)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            return Err(io::Error::other(
                "No display server found. Set DISPLAY (X11) or WAYLAND_DISPLAY environment variable."
            ));
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([800.0, 600.0])
        .with_title("Clay Mud Client")
        .with_transparent(true);

    // Set window icon from embedded PNG
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        ..Default::default()
    };

    let addr_string = addr.to_string();

    eframe::run_native(
        "Clay Mud Client",
        options,
        Box::new(move |cc| {
            // Install image loaders for Discord emoji support
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::new(RemoteGuiApp::new(addr_string, runtime)) as Box<dyn eframe::App>
        }),
    ).map_err(|e| io::Error::other(format!("eframe error: {}", e)))
}

pub fn run_remote_gui(addr: &str) -> std::io::Result<()> {
    let runtime = tokio::runtime::Handle::current();
    run(addr, runtime)
}

/// Run the master GUI mode: App runs in-process on tokio, GUI on the main thread.
pub fn run_master_gui() -> std::io::Result<()> {
    // Check for display server availability (Linux only - Windows/macOS always have a display)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            return Err(std::io::Error::other(
                "No display server found. Set DISPLAY (X11) or WAYLAND_DISPLAY environment variable."
            ));
        }
    }

    // Build a multi-threaded tokio runtime for the App
    let runtime = tokio::runtime::Runtime::new()?;
    let handle = runtime.handle().clone();

    // Create bidirectional channels between App and GUI
    let (app_to_gui_tx, app_to_gui_rx) = mpsc::unbounded_channel::<crate::WsMessage>();
    let (gui_to_app_tx, gui_to_app_rx) = mpsc::unbounded_channel::<crate::WsMessage>();

    // Spawn the headless App on the tokio runtime
    handle.spawn(async move {
        if let Err(e) = crate::run_app_headless(app_to_gui_tx, gui_to_app_rx).await {
            eprintln!("App error: {}", e);
        }
    });

    // Run the GUI on the main thread (required by eframe/windowing systems)
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([800.0, 600.0])
        .with_title("Clay Mud Client")
        .with_transparent(true);

    // Set window icon from embedded PNG
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        ..Default::default()
    };

    let result = eframe::run_native(
        "Clay Mud Client",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::new(RemoteGuiApp::new_master(app_to_gui_rx, gui_to_app_tx, handle))
                as Box<dyn eframe::App>
        }),
    ).map_err(|e| std::io::Error::other(format!("eframe error: {}", e)));

    // Shut down tokio runtime on a background thread to avoid panic when
    // dropping it after eframe's event loop exits (blocking is not allowed
    // in that context)
    runtime.shutdown_background();

    result
}
