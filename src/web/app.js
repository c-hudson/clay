// Clay MUD Client - Web Interface

(function() {
    'use strict';

    // Maximum line length to prevent performance issues with extremely long lines
    const MAX_LINE_LENGTH = 10000;

    // Truncate text if it exceeds MAX_LINE_LENGTH
    function truncateIfNeeded(text) {
        if (text.length > MAX_LINE_LENGTH) {
            return text.substring(0, MAX_LINE_LENGTH) + '\x1b[0m\x1b[33m... [truncated]\x1b[0m';
        }
        return text;
    }

    // DOM elements
    const elements = {
        output: document.getElementById('output'),
        outputContainer: document.getElementById('output-container'),
        statusDot: document.getElementById('status-dot'),
        worldName: document.getElementById('world-name'),
        statusMore: document.getElementById('status-more'),
        moreLabel: document.getElementById('more-label'),
        moreCount: document.getElementById('more-count'),
        activityIndicator: document.getElementById('activity-indicator'),
        activityCount: document.getElementById('activity-count'),
        statusTime: document.getElementById('status-time'),
        statusBar: document.getElementById('status-bar'),
        inputContainer: document.getElementById('input-container'),
        prompt: document.getElementById('prompt'),
        input: document.getElementById('input'),
        sendBtn: document.getElementById('send-btn'),
        authModal: document.getElementById('auth-modal'),
        authPrompt: document.getElementById('auth-prompt'),
        authUsernameRow: document.getElementById('auth-username-row'),
        authUsername: document.getElementById('auth-username'),
        authPassword: document.getElementById('auth-password'),
        authKeyRow: document.getElementById('auth-key-row'),
        authKeyInput: document.getElementById('auth-key'),
        authError: document.getElementById('auth-error'),
        authSubmit: document.getElementById('auth-submit'),
        // Connection error modal
        connectionErrorModal: document.getElementById('connection-error-modal'),
        connectionErrorText: document.getElementById('connection-error-text'),
        connectionRetryBtn: document.getElementById('connection-retry-btn'),
        connectionCancelBtn: document.getElementById('connection-cancel-btn'),
        // Reconnect modal (shown when send fails)
        reconnectModal: document.getElementById('reconnect-modal'),
        reconnectText: document.getElementById('reconnect-text'),
        reconnectBtn: document.getElementById('reconnect-btn'),
        reconnectCancelBtn: document.getElementById('reconnect-cancel-btn'),
        // Device mode selector (long-press menu)
        deviceModeModal: document.getElementById('device-mode-modal'),
        deviceModeList: document.getElementById('device-mode-list'),
        // Password change modal (multiuser mode)
        passwordModal: document.getElementById('password-modal'),
        passwordOld: document.getElementById('password-old'),
        passwordNew: document.getElementById('password-new'),
        passwordConfirm: document.getElementById('password-confirm'),
        passwordError: document.getElementById('password-error'),
        passwordSaveBtn: document.getElementById('password-save-btn'),
        passwordCancelBtn: document.getElementById('password-cancel-btn'),
        connectingOverlay: document.getElementById('connecting-overlay'),
        // Menu
        menuBtn: document.getElementById('menu-btn'),
        menuDropdown: document.getElementById('menu-dropdown'),
        // Font slider (status bar)
        fontSliderInput: document.getElementById('font-slider'),
        fontSliderVal: document.getElementById('font-slider-val'),
        // Nav bar (tablet/phone)
        navBar: document.getElementById('nav-bar'),
        navMenuBtn: document.getElementById('nav-menu-btn'),
        navPgUpBtn: document.getElementById('nav-pgup-btn'),
        navPgDnBtn: document.getElementById('nav-pgdn-btn'),
        navUpBtn: document.getElementById('nav-up-btn'),
        navDownBtn: document.getElementById('nav-down-btn'),
        navFontSlider: document.getElementById('nav-font-slider'),
        navFontSliderVal: document.getElementById('nav-font-slider-val'),
        // Actions List popup
        actionsListModal: document.getElementById('actions-list-modal'),
        actionFilter: document.getElementById('action-filter'),
        actionWorldFilterIndicator: document.getElementById('action-world-filter'),
        actionsList: document.getElementById('actions-list'),
        actionAddBtn: document.getElementById('action-add-btn'),
        actionEditBtn: document.getElementById('action-edit-btn'),
        actionDeleteBtn: document.getElementById('action-delete-btn'),
        actionCancelBtn: document.getElementById('action-cancel-btn'),
        actionsListCloseBtn: document.getElementById('actions-list-close-btn'),
        // Actions Editor popup
        actionsEditorModal: document.getElementById('actions-editor-modal'),
        actionEditorTitle: document.getElementById('action-editor-title'),
        actionName: document.getElementById('action-name'),
        actionWorld: document.getElementById('action-world'),
        actionMatchType: document.getElementById('action-match-type'),
        actionPattern: document.getElementById('action-pattern'),
        actionCommand: document.getElementById('action-command'),
        actionEnabled: document.getElementById('action-enabled'),
        actionStartup: document.getElementById('action-startup'),
        actionError: document.getElementById('action-error'),
        actionSaveBtn: document.getElementById('action-save-btn'),
        actionEditorDeleteBtn: document.getElementById('action-editor-delete-btn'),
        actionEditorCancelBtn: document.getElementById('action-editor-cancel-btn'),
        actionsEditorCloseBtn: document.getElementById('actions-editor-close-btn'),
        // Actions Confirm Delete popup
        actionConfirmModal: document.getElementById('action-confirm-modal'),
        actionConfirmText: document.getElementById('action-confirm-text'),
        actionConfirmYesBtn: document.getElementById('action-confirm-yes-btn'),
        actionConfirmNoBtn: document.getElementById('action-confirm-no-btn'),
        // Worlds list popup
        worldsModal: document.getElementById('worlds-modal'),
        worldsTableBody: document.getElementById('worlds-table-body'),
        worldsCloseBtn: document.getElementById('worlds-close-btn'),
        worldsListCloseBtn: document.getElementById('worlds-list-close-btn'),
        // World selector popup
        worldSelectorModal: document.getElementById('world-selector-modal'),
        worldFilter: document.getElementById('world-filter'),
        worldSelectorTableBody: document.getElementById('world-selector-table-body'),
        worldSelectorOnlyConnected: document.getElementById('world-selector-only-connected'),
        worldAddBtn: document.getElementById('world-add-btn'),
        worldEditBtn: document.getElementById('world-edit-btn'),
        worldConnectBtn: document.getElementById('world-connect-btn'),
        worldSelectorCancelBtn: document.getElementById('world-selector-cancel-btn'),
        // World delete confirm popup
        worldConfirmModal: document.getElementById('world-confirm-modal'),
        worldConfirmText: document.getElementById('world-confirm-text'),
        worldConfirmYesBtn: document.getElementById('world-confirm-yes-btn'),
        worldConfirmNoBtn: document.getElementById('world-confirm-no-btn'),
        // World editor popup
        worldEditorModal: document.getElementById('world-editor-modal'),
        worldEditorTitle: document.getElementById('world-editor-title'),
        worldEditName: document.getElementById('world-edit-name'),
        worldEditHostname: document.getElementById('world-edit-hostname'),
        worldEditPort: document.getElementById('world-edit-port'),
        worldEditUser: document.getElementById('world-edit-user'),
        worldEditPassword: document.getElementById('world-edit-password'),
        worldEditSslToggle: document.getElementById('world-edit-ssl-toggle'),
        worldEditAutoLoginSelect: document.getElementById('world-edit-auto-login-select'),
        worldEditKeepAliveSelect: document.getElementById('world-edit-keep-alive-select'),
        worldEditKeepAliveCmdField: document.getElementById('world-edit-keep-alive-cmd-field'),
        worldEditKeepAliveCmd: document.getElementById('world-edit-keep-alive-cmd'),
        worldEditEncodingSelect: document.getElementById('world-edit-encoding-select'),
        worldEditLoggingToggle: document.getElementById('world-edit-logging-toggle'),
        worldEditGmcpPackages: document.getElementById('world-edit-gmcp-packages'),
        worldEditAutoReconnect: document.getElementById('world-edit-auto-reconnect'),
        worldEditCloseBtn: document.getElementById('world-edit-close-btn'),
        worldEditDeleteBtn: document.getElementById('world-edit-delete-btn'),
        worldEditCancelBtn: document.getElementById('world-edit-cancel-btn'),
        worldEditSaveBtn: document.getElementById('world-edit-save-btn'),
        worldEditConnectBtn: document.getElementById('world-edit-connect-btn'),
        // Web settings fields (inside combined settings modal)
        webProtocolSelect: document.getElementById('web-protocol-select'),
        webHttpEnabledSelect: document.getElementById('web-http-enabled-select'),
        webHttpPort: document.getElementById('web-http-port'),
        webAllowList: document.getElementById('web-allow-list'),
        webWsPassword: document.getElementById('web-ws-password'),
        webCertFile: document.getElementById('web-cert-file'),
        webKeyFile: document.getElementById('web-key-file'),
        tlsCertField: document.getElementById('tls-cert-field'),
        tlsKeyField: document.getElementById('tls-key-field'),
        httpLabel: document.getElementById('http-label'),
        httpPortLabel: document.getElementById('http-port-label'),
        // wsLabel removed — WS shares HTTP port
        // wsPortLabel removed — WS shares HTTP port
        // Combined settings popup (/setup + /web)
        settingsModal: document.getElementById('settings-modal'),
        settingsCloseBtn: document.getElementById('settings-close-btn'),
        settingsSaveBtn: document.getElementById('settings-save-btn'),
        settingsCancelBtn: document.getElementById('settings-cancel-btn'),
        settingsHelpBtn: document.getElementById('settings-help-btn'),
        settingsTitle: document.getElementById('settings-title'),
        settingsGeneralSection: document.getElementById('settings-general'),
        settingsWebSection: document.getElementById('settings-web'),
        settingsClayServerSection: document.getElementById('settings-clay-server'),
        webAuthKey: document.getElementById('web-auth-key'),
        webAuthKeyRegen: document.getElementById('web-auth-key-regen'),
        // Setup fields (inside combined settings modal)
        setupMoreModeToggle: document.getElementById('setup-more-mode-toggle'),
        setupAnsiMusicToggle: document.getElementById('setup-ansi-music-toggle'),
        setupZwjToggle: document.getElementById('setup-zwj-toggle'),
        setupTtsSelect: document.getElementById('setup-tts-select'),
        setupTtsSpeakModeSelect: document.getElementById('setup-tts-speak-mode-select'),
        setupTlsProxyToggle: document.getElementById('setup-tls-proxy-toggle'),
        setupNewLineIndicatorToggle: document.getElementById('setup-new-line-indicator-toggle'),
        setupDebugToggle: document.getElementById('setup-debug-toggle'),
        setupWorldSwitchSelect: document.getElementById('setup-world-switch-select'),
        setupInputHeightValue: document.getElementById('setup-input-height-value'),
        setupHeightMinus: document.getElementById('setup-height-minus'),
        setupHeightPlus: document.getElementById('setup-height-plus'),
        setupColorOffsetValue: document.getElementById('setup-color-offset-value'),
        setupColorOffsetMinus: document.getElementById('setup-color-offset-minus'),
        setupColorOffsetPlus: document.getElementById('setup-color-offset-plus'),
        setupThemeSelect: document.getElementById('setup-theme-select'),
        setupTransparencyRow: document.getElementById('setup-transparency-row'),
        setupTransparencySlider: document.getElementById('setup-transparency-slider'),
        setupTransparencyValue: document.getElementById('setup-transparency-value'),
        // Filter popup (F4)
        filterPopup: document.getElementById('filter-popup'),
        filterInput: document.getElementById('filter-input'),
        // Help popup (/help)
        helpModal: document.getElementById('help-modal'),
        helpContent: document.getElementById('help-content'),
        helpCloseBtn: document.getElementById('help-close-btn'),
        helpOkBtn: document.getElementById('help-ok-btn'),
        // Menu popup (/menu)
        menuModal: document.getElementById('menu-modal'),
        menuList: document.getElementById('menu-list'),
        // Font fields (inside combined settings modal)
        settingsFontSection: document.getElementById('settings-font'),
        fontFamilyList: document.getElementById('font-family-list'),
        fontPhoneMinus: document.getElementById('font-phone-minus'),
        fontPhonePlus: document.getElementById('font-phone-plus'),
        fontPhoneValue: document.getElementById('font-phone-value'),
        fontTabletMinus: document.getElementById('font-tablet-minus'),
        fontTabletPlus: document.getElementById('font-tablet-plus'),
        fontTabletValue: document.getElementById('font-tablet-value'),
        fontDesktopMinus: document.getElementById('font-desktop-minus'),
        fontDesktopPlus: document.getElementById('font-desktop-plus'),
        fontDesktopValue: document.getElementById('font-desktop-value'),
        fontWeightMinus: document.getElementById('font-weight-minus'),
        fontWeightPlus: document.getElementById('font-weight-plus'),
        fontWeightValue: document.getElementById('font-weight-value'),
        fontAdvancedToggle: document.getElementById('font-advanced-toggle'),
        fontAdvancedSection: document.getElementById('font-advanced-section'),
        fontLineheightMinus: document.getElementById('font-lineheight-minus'),
        fontLineheightPlus: document.getElementById('font-lineheight-plus'),
        fontLineheightValue: document.getElementById('font-lineheight-value'),
        fontLetterspacingMinus: document.getElementById('font-letterspacing-minus'),
        fontLetterspacingPlus: document.getElementById('font-letterspacing-plus'),
        fontLetterspacingValue: document.getElementById('font-letterspacing-value'),
        fontWordspacingMinus: document.getElementById('font-wordspacing-minus'),
        fontWordspacingPlus: document.getElementById('font-wordspacing-plus'),
        fontWordspacingValue: document.getElementById('font-wordspacing-value'),
        // Popup help modal (shared)
        popupHelpModal: document.getElementById('popup-help-modal'),
        popupHelpContent: document.getElementById('popup-help-content'),
        popupHelpCloseBtn: document.getElementById('popup-help-close-btn'),
        popupHelpOkBtn: document.getElementById('popup-help-ok-btn'),
        // Help buttons in each popup (settings-help-btn in combined modal, referenced as settingsHelpBtn above)
        worldEditHelpBtn: document.getElementById('world-edit-help-btn'),
        worldSelectorHelpBtn: document.getElementById('world-selector-help-btn'),
        actionsListHelpBtn: document.getElementById('actions-list-help-btn'),
        actionEditorHelpBtn: document.getElementById('action-editor-help-btn'),
        connectionsHelpBtn: document.getElementById('connections-help-btn'),
        menuHelpBtn: document.getElementById('menu-help-btn')
    };

    // State
    let ws = null;
    let authenticated = false;
    let multiuserMode = false;  // True when server is in multiuser mode
    let pendingAuthPassword = null;  // Password being authenticated (saved on success for Android auto-login)
    let pendingAuthUsername = null;  // Username being authenticated (saved on success for Android auto-login)
    let deferredAutoLoginPassword = null;  // Saved password waiting for ServerHello
    let deferredAutoLoginUsername = null;  // Saved username waiting for ServerHello
    let authKey = null;  // Device auth key for passwordless authentication
    let authKeyPending = false;  // True when trying key-based auth (to fall back to password on failure)
    let keyAuthFailed = false;   // Set after key rejection so reconnect skips key auth and shows password prompt
    let serverChallenge = '';  // Challenge from ServerHello for challenge-response auth
    let hasReceivedInitialState = false;  // True after first InitialState (to preserve world on resync)
    let worlds = [];
    let currentWorldIndex = 0;

    // Check for world lock parameter in URL or injected by WebView
    var urlParams = new URLSearchParams(window.location.search);
    var lockedWorldName = urlParams.get('world') || window.LOCK_WORLD || null;
    var lockedWorld = false;

    // Grep mode: filter output by pattern (set by /window --grep or URL ?grep=)
    var grepMode = null;
    var grepRegex = null;
    if (window.GREP_MODE) {
        grepMode = window.GREP_MODE;
    } else if (urlParams.get('grep')) {
        grepMode = {
            pattern: urlParams.get('grep'),
            regex: urlParams.get('regexp') === '1'
        };
        var grepWorldParam = urlParams.get('world');
        if (grepWorldParam) {
            lockedWorldName = grepWorldParam;
        }
    }
    if (grepMode) {
        try {
            if (grepMode.regex) {
                grepRegex = new RegExp(grepMode.pattern, 'i');
            } else {
                // Convert glob to regex: * → .*, ? → ., escape rest
                var escaped = grepMode.pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&');
                escaped = escaped.replace(/\*/g, '.*').replace(/\?/g, '.');
                grepRegex = new RegExp(escaped, 'i');
            }
        } catch (e) {
            // Invalid pattern — match everything
            grepRegex = null;
        }
    }
    let pendingReconnectCommand = null;  // Command to resend after reconnect
    let pendingReconnectWorldIndex = null;  // World index to switch to after reconnect
    let commandHistory = [];
    let historyIndex = -1;
    let connectionFailures = 0;
    let reloadReconnect = false;
    let reloadReconnectAttempts = 0;
    let inputHeight = 1;
    let splashLines = [];  // Splash screen lines for multiuser mode

    // Lazy backfill state
    let backfillInProgress = false;
    let backfillWorldQueue = [];
    let backfillCurrentWorld = null;
    const BACKFILL_CHUNK_SIZE = 500;
    const BACKFILL_DELAY_MS = 200;

    // Cached rendered output per world (array of DOM elements)
    let worldOutputCache = [];

    // Partial line buffer per world (for handling split lines across reads)
    let partialLines = {};

    // More-mode state (per world)
    let moreModeEnabled = true;
    let paused = false;
    let pendingLines = [];
    let linesSincePause = 0;

    // Synchronized more-mode: track last sent view state to avoid redundant messages
    let lastSentViewState = null;  // {worldIndex, visibleLines}

    // Server's activity count (number of worlds with unseen/pending output)
    let serverActivityCount = 0;

    // Settings
    let worldSwitchMode = 'Unseen First';  // 'Unseen First' or 'Alphabetical'
    let keybindings = {};  // key name -> action ID, received from server
    let killRing = [];     // killed text for yank (Ctrl+Y)

    // Actions state
    let actions = [];
    let actionsListPopupOpen = false;
    let actionsEditorPopupOpen = false;
    let actionsConfirmPopupOpen = false;
    let selectedActionIndex = -1;
    let editingActionIndex = -1;  // -1 = new action, >=0 = editing existing
    let actionsWorldFilter = '';  // Filter by world from /actions <world>

    // Tag display state
    let showTags = false;
    let highlightActions = false;

    // Color offset percentage (0 = disabled, 1-100 = adjustment percentage)
    let colorOffsetPercent = 0;

    // Command completion state
    let lastCompletionPrefix = '';
    let lastCompletionIndex = -1;

    // World popup state
    let worldsPopupOpen = false;
    let worldSelectorPopupOpen = false;
    let worldConfirmPopupOpen = false;
    let worldSelectorOnlyConnected = false;
    let worldEditorPopupOpen = false;
    let worldEditorIndex = -1;  // Index of world being edited

    // Web settings popup state (global state from server)
    let settingsPopupOpen = false;
    let settingsActiveTab = 'general';
    let webSecure = false;
    let httpEnabled = false;
    let httpPort = 9000;
    let wsEnabled = false;
    let wsPort = 9001;
    let wsAllowList = '';
    let wsCertFile = '';
    let wsKeyFile = '';
    let wsPassword = '';
    let tlsConfigured = false;  // True if server has TLS cert+key configured
    let serverAuthKey = '';  // Auth key from server (for display in web settings)
    // Temporary editing state for web popup (only saved on Save button)
    let editWebSecure = false;
    let editHttpEnabled = false;
    let editWsEnabled = false;
    let selectedWorldIndex = -1;
    let selectedWorldsRowIndex = -1; // For worlds list popup (/connections)

    // Setup popup state
    // setupPopupOpen removed — merged into settingsPopupOpen
    let setupMoreMode = true;
    let setupWorldSwitchMode = 'Unseen First';
    // Note: show tags removed from setup - controlled by F2 or /tag command
    let setupColorOffset = 0;
    let setupAnsiMusic = true;
    let setupZwj = false;
    let setupTtsMode = 'Off';
    let setupTlsProxy = false;
    let setupNewLineIndicator = false;
    let setupDebug = false;
    let setupInputHeightValue = 1;
    let setupGuiTheme = 'dark';
    let setupTransparency = 1.0;

    // Filter popup state (F4)
    let filterPopupOpen = false;
    let filterText = '';

    // Font popup state (/font)
    // fontPopupOpen removed — merged into settingsPopupOpen
    let fontName = '';  // Shared font family name (synced from server)
    let guiFontSize = 14.0;  // GUI font size (not used by web, but preserved for server)
    let fontEditName = '';
    let fontEditSizePhone = 10;
    let fontEditSizeTablet = 14;
    let fontEditSizeDesktop = 18;
    let webFontWeight = 400;
    let fontEditWeight = 400;
    let webFontLineHeight = 1.2;
    let webFontLetterSpacing = 0;
    let webFontWordSpacing = 0;
    let fontEditLineHeight = 1.2;
    let fontEditLetterSpacing = 0;
    let fontEditWordSpacing = 0;

    // Font families (matching remote GUI FONT_FAMILIES)
    const FONT_FAMILIES = [
        ['', 'System Default'],
        ['Monospace', 'Monospace'],
        ['DejaVu Sans Mono', 'DejaVu Sans Mono'],
        ['Liberation Mono', 'Liberation Mono'],
        ['Ubuntu Mono', 'Ubuntu Mono'],
        ['Fira Code', 'Fira Code'],
        ['Source Code Pro', 'Source Code Pro'],
        ['JetBrains Mono', 'JetBrains Mono'],
        ['Hack', 'Hack'],
        ['Inconsolata', 'Inconsolata'],
        ['Courier New', 'Courier New'],
        ['Consolas', 'Consolas'],
    ];

    // Help popup state (/help)
    let helpPopupOpen = false;

    // Menu popup state (/menu)
    let menuPopupOpen = false;
    let menuSelectedIndex = 0;
    const menuItems = [
        { label: 'Help', command: '/help' },
        { label: 'Settings', command: '/setup' },
        { label: 'Web Settings', command: '/web' },
        { label: 'Font', command: '/font' },
        { label: 'Actions', command: '/actions' },
        { label: 'World Selector', command: '/worlds' },
        { label: 'Connected Worlds', command: '/connections' }
    ];

    // Current theme values (synced from server)
    let consoleTheme = 'dark';
    let guiTheme = 'dark';

    // Menu state
    let menuOpen = false;

    // Font size state: pixel value (9-20 range)
    let currentFontSize = 14;  // Default to 14px

    // Per-device font size tracking (saved separately for phone/tablet/desktop)
    let deviceType = 'desktop';  // 'phone', 'tablet', or 'desktop'
    let deviceModeOverride = window.WEBVIEW_DEVICE_OVERRIDE || null;  // null = auto, or 'phone', 'tablet', 'desktop'
    let webFontSizePhone = 10.0;
    let webFontSizeTablet = 14.0;
    let webFontSizeDesktop = 18.0;

    // Clamp font size to valid range
    function clampFontSize(px) {
        return Math.max(9, Math.min(20, Math.round(px)));
    }

    // Device mode: 'desktop', 'tablet', or 'phone'
    let deviceMode = 'desktop';

    // ANSI Music audio context (lazily initialized)
    let audioContext = null;
    let ansiMusicEnabled = true;  // Will be synced from server settings
    let zwjEnabled = false;  // Will be synced from server settings
    let ttsMode = 'off';  // Will be synced from server settings ('off', 'local', 'edge')
    let ttsSpeakMode = 'all';  // 'all' or 'limit'
    let newLineIndicator = false;  // Will be synced from server settings

    // MCMP (MUD Client Media Protocol) state
    let mcmpDefaultUrl = '';
    let mcmpMusicPlayer = null;    // { audio, key, name } - one music track at a time
    let mcmpSoundPlayers = {};     // key -> { audio, name }
    let mcmpMusicFadeTimer = null;

    let tlsProxyEnabled = false;  // TLS proxy for connection preservation over hot reload
    let tempConvertEnabled = false;  // Temperature conversion (32F -> 32F(0C))
    let mouseEnabled = true;  // Console mouse support
    let debugEnabled = false;  // Debug logging
    let dictionaryPath = '';  // Custom dictionary path
    let spellCheckEnabled = true;  // Spell checking
    let prevInputLen = 0;  // Track previous input length for temp conversion
    let skipTempConversion = null;  // Temperature to skip re-converting (after user undid conversion)

    // ============================================================================
    // Theme Application
    // ============================================================================

    // Apply theme to the document body
    function applyTheme(theme) {
        if (theme === 'light') {
            document.body.classList.add('theme-light');
        } else {
            document.body.classList.remove('theme-light');
        }
    }

    // Apply theme colors from JSON (updates CSS custom properties in the DOM)
    function applyThemeColors(jsonStr) {
        try {
            const colors = JSON.parse(jsonStr);
            const el = document.getElementById('theme-vars');
            if (!el) return;
            let css = ':root { ';
            for (const [key, val] of Object.entries(colors)) {
                css += '--theme-' + key.replace(/[_.]/g, '-') + ': ' + val + '; ';
            }
            css += '}';
            el.textContent = css;
            // Re-apply window opacity for webview mode
            if (window.WEBVIEW_MODE) applyTransparency(guiTransparency);
        } catch (e) {
            // ignore parse errors
        }
    }

    // Apply window transparency (webview mode only)
    // Uses GTK window opacity via IPC — sets _NET_WM_WINDOW_OPACITY on X11.
    // This is compositor-managed (instant, reliable), unlike per-pixel alpha through
    // WebKit2GTK's rendering pipeline which has timing/ghosting issues.
    let guiTransparency = 1.0;
    function applyTransparency(alpha) {
        guiTransparency = alpha;
        if (!window.WEBVIEW_MODE || !window.ipc) return;
        window.ipc.postMessage('opacity:' + alpha);
    }

    // Apply font family to the interface
    function applyFontFamily(name) {
        fontName = name;
        if (name && name !== '') {
            document.documentElement.style.setProperty('--mono-override', "'" + name + "', var(--mono)");
        } else {
            document.documentElement.style.setProperty('--mono-override', 'var(--mono)');
        }
        // Apply to elements that use monospace fonts
        const monoStyle = name && name !== '' ? "'" + name + "', var(--mono)" : '';
        elements.output.style.fontFamily = monoStyle || '';
        elements.input.style.fontFamily = monoStyle || '';
        if (elements.prompt) elements.prompt.style.fontFamily = monoStyle || '';
    }

    function applyFontWeight(w) {
        document.body.style.fontWeight = w;
    }

    function applyAdvancedFontSettings() {
        var output = elements.output;
        var input = elements.input;
        output.style.lineHeight = webFontLineHeight;
        input.style.lineHeight = webFontLineHeight;
        output.style.letterSpacing = webFontLetterSpacing ? webFontLetterSpacing + 'px' : '';
        input.style.letterSpacing = webFontLetterSpacing ? webFontLetterSpacing + 'px' : '';
        output.style.wordSpacing = webFontWordSpacing ? webFontWordSpacing + 'px' : '';
        input.style.wordSpacing = webFontWordSpacing ? webFontWordSpacing + 'px' : '';
    }

    // ============================================================================
    // Command Definitions (single source of truth is Rust's parse_command)
    // ============================================================================

    // Internal commands for tab completion (must match Rust parse_command match arms)
    // This list is verified by test_command_parity_js_vs_rust in main.rs
    const INTERNAL_COMMANDS = [
        'help', 'version', 'quit', 'reload', 'update', 'setup', 'web', 'actions',
        'worlds', 'world', 'connections', 'l', 'disconnect', 'dc',
        'flush', 'menu', 'send', 'remote', 'ban', 'unban',
        'testmusic', 'dump', 'notify', 'addworld', 'note', 'tag', 'tags',
        'dict', 'urban', 'translate', 'tr', 'font', 'window',
    ];

    function isInternalCommand(name) {
        return INTERNAL_COMMANDS.includes(name.toLowerCase());
    }

    // Command completion - returns completed command or null if no match
    function completeCommand(input) {
        if (!input.startsWith('/')) return null;

        // Get the partial command (everything up to first space)
        const spacePos = input.indexOf(' ');
        const partial = spacePos >= 0 ? input.substring(0, spacePos) : input;
        const args = spacePos >= 0 ? input.substring(spacePos) : '';

        // Only complete if cursor is in the command part
        if (spacePos >= 0 && elements.input.selectionStart > spacePos) {
            return null;
        }

        // Build list of completions: internal commands + manual actions
        let completions = INTERNAL_COMMANDS.map(cmd => '/' + cmd);

        // Add manual actions (empty pattern)
        const manualActions = actions
            .filter(a => !a.pattern || a.pattern.trim() === '')
            .map(a => '/' + a.name);
        completions = completions.concat(manualActions);

        // Find all matches
        const partialLower = partial.toLowerCase();
        let matches = completions.filter(cmd => cmd.toLowerCase().startsWith(partialLower));

        if (matches.length === 0) return null;

        // Sort and dedupe
        matches.sort();
        matches = [...new Set(matches)];

        // Check if this is a continuation of previous completion
        let nextIndex = 0;
        if (partial.toLowerCase() === lastCompletionPrefix.toLowerCase()) {
            // Cycle to next match
            nextIndex = (lastCompletionIndex + 1) % matches.length;
        } else {
            // Find current match if we're already on a completed command
            const currentIdx = matches.findIndex(m => m.toLowerCase() === partial.toLowerCase());
            if (currentIdx >= 0) {
                nextIndex = (currentIdx + 1) % matches.length;
            }
        }

        // Update completion state
        lastCompletionPrefix = partial;
        lastCompletionIndex = nextIndex;

        // Return the completion with preserved arguments
        return matches[nextIndex] + args;
    }

    // Reset completion state (call when input changes by typing)
    function resetCompletion() {
        lastCompletionPrefix = '';
        lastCompletionIndex = -1;
    }

    // Check for temperature patterns and convert them
    // Patterns: 32F, 32f, 100C, 100c, 32°F, 32.5F, -10C, etc.
    // When detected, inserts conversion in parentheses: "32F " -> "32F(0C) "
    function checkTempConversion() {
        // Only convert when enabled
        if (!tempConvertEnabled) return;

        const input = elements.input.value;
        if (!input || input.length === 0) {
            prevInputLen = 0;
            return;
        }

        // Don't convert when user is deleting - allows undoing conversion
        if (input.length <= prevInputLen) {
            prevInputLen = input.length;
            return;
        }
        prevInputLen = input.length;

        // Only check when cursor is at the end
        if (elements.input.selectionStart !== input.length) return;

        const lastChar = input[input.length - 1];
        // Only trigger on separator characters
        if (!/[\s.,!?;:\)\]\}]/.test(lastChar)) {
            // Non-separator typed - clear skip so next temperature can convert
            skipTempConversion = null;
            return;
        }

        // Pattern: optional minus, digits, optional decimal+digits, optional °, F or C
        // Look for temp pattern just before the separator
        const match = input.slice(0, -1).match(/(-?\d+\.?\d*)(°?[FfCc])$/);
        if (!match) return;

        // Make sure it's not part of a word (check char before the number)
        const numStart = input.length - 1 - match[0].length;
        if (numStart > 0) {
            const prevChar = input[numStart - 1];
            if (/[a-zA-Z0-9_]/.test(prevChar)) return;
        }

        // Build the full temperature string (e.g., "21F", "-5.5°C")
        const tempStr = match[0];

        // Check if this temperature was already converted and undone - skip if so
        if (skipTempConversion === tempStr) {
            return;
        }

        const tempValue = parseFloat(match[1]);
        const unit = match[2].toUpperCase().replace('°', '');
        if (isNaN(tempValue)) return;

        let converted, convertedUnit;
        if (unit === 'F') {
            // Fahrenheit to Celsius
            converted = (tempValue - 32) * 5 / 9;
            convertedUnit = 'C';
        } else {
            // Celsius to Fahrenheit
            converted = tempValue * 9 / 5 + 32;
            convertedUnit = 'F';
        }

        // Format the conversion - integer if whole, else one decimal
        // No space before the parenthesis - the separator the user typed goes after
        const convertedStr = Math.abs(converted - Math.round(converted)) < 0.05
            ? `(${Math.round(converted)}${convertedUnit})`
            : `(${converted.toFixed(1)}${convertedUnit})`;

        // Remember this temperature so we don't re-convert if user undoes it
        skipTempConversion = tempStr;

        // Insert conversion before the separator
        const beforeSep = input.slice(0, -1);
        const sep = lastChar;
        elements.input.value = beforeSep + convertedStr + sep;
        // Update prevInputLen to reflect new length after conversion
        prevInputLen = elements.input.value.length;
        // Move cursor to end
        elements.input.selectionStart = elements.input.selectionEnd = elements.input.value.length;
    }

    // Command parsing is handled server-side by Rust's parse_command().
    // Web client sends all commands to the server via SendCommand message.
    // Server responds with ExecuteLocalCommand for UI/popup commands.

    // ============================================================================
    // Device Detection
    // ============================================================================

    // Detect device type and return appropriate font size position (0-3)
    // Also sets the global deviceType variable ('phone', 'tablet', 'desktop')
    function detectDeviceType() {
        // If override is set, use that instead of auto-detection
        if (deviceModeOverride) {
            deviceType = deviceModeOverride;
            if (deviceModeOverride === 'phone') {
                return { fontSize: clampFontSize(webFontSizePhone), mode: 'phone', device: 'phone' };
            } else if (deviceModeOverride === 'tablet') {
                return { fontSize: clampFontSize(webFontSizeTablet), mode: 'tablet', device: 'tablet' };
            } else {
                return { fontSize: clampFontSize(webFontSizeDesktop), mode: 'desktop', device: 'desktop' };
            }
        }

        const width = window.innerWidth;
        const hasTouch = 'ontouchstart' in window || navigator.maxTouchPoints > 0;

        // Phone: narrow screen (< 768px)
        if (width < 768) {
            deviceType = 'phone';
            return { fontSize: clampFontSize(webFontSizePhone), mode: 'phone', device: 'phone' };
        }
        // Tablet: medium screen with touch (768-1024px)
        if (width <= 1024 && hasTouch) {
            deviceType = 'tablet';
            return { fontSize: clampFontSize(webFontSizeTablet), mode: 'tablet', device: 'tablet' };
        }
        // Desktop: wide screen or no touch
        deviceType = 'desktop';
        return { fontSize: clampFontSize(webFontSizeDesktop), mode: 'desktop', device: 'desktop' };
    }

    // Helper to focus input and ensure keyboard shows on mobile
    function focusInputWithKeyboard() {
        elements.input.focus();
        // On Android, sometimes need to set selection to trigger keyboard
        if (deviceMode === 'phone' || deviceMode === 'tablet') {
            const len = elements.input.value.length;
            elements.input.setSelectionRange(len, len);
        }
    }

    // Custom dropdown for mobile (replaces native select with styled dropdown)
    let activeCustomDropdown = null;

    function initCustomDropdowns() {
        document.querySelectorAll('select.form-select').forEach(select => {
            // Create wrapper
            const wrapper = document.createElement('div');
            wrapper.className = 'custom-dropdown';

            // Create the visible button that shows current value
            const button = document.createElement('div');
            button.className = 'custom-dropdown-button';
            button.textContent = select.options[select.selectedIndex]?.text || '';

            // Create dropdown menu
            const menu = document.createElement('div');
            menu.className = 'custom-dropdown-menu';

            // Populate options
            Array.from(select.options).forEach((option, index) => {
                const item = document.createElement('div');
                item.className = 'custom-dropdown-item';
                if (index === select.selectedIndex) {
                    item.classList.add('selected');
                }
                item.textContent = option.text;
                item.dataset.value = option.value;
                item.onclick = (e) => {
                    e.stopPropagation();
                    select.value = option.value;
                    button.textContent = option.text;
                    menu.querySelectorAll('.custom-dropdown-item').forEach(i => i.classList.remove('selected'));
                    item.classList.add('selected');
                    closeCustomDropdown();
                    // Trigger change event on the original select
                    select.dispatchEvent(new Event('change'));
                };
                menu.appendChild(item);
            });

            // Insert wrapper and move select inside (hidden)
            select.parentNode.insertBefore(wrapper, select);
            wrapper.appendChild(button);
            wrapper.appendChild(menu);
            wrapper.appendChild(select);
            select.style.display = 'none';

            // Toggle dropdown on button click
            button.onclick = (e) => {
                e.stopPropagation();
                if (menu.classList.contains('visible')) {
                    closeCustomDropdown();
                } else {
                    // Close any other open dropdown
                    closeCustomDropdown();
                    menu.classList.add('visible');
                    activeCustomDropdown = menu;
                }
            };

            // Store reference for updating
            select._customButton = button;
            select._customMenu = menu;
        });

        // Close dropdown when clicking outside
        document.addEventListener('click', closeCustomDropdown);
    }

    function closeCustomDropdown() {
        if (activeCustomDropdown) {
            activeCustomDropdown.classList.remove('visible');
            activeCustomDropdown = null;
        }
    }

    // Update custom dropdown when select value changes programmatically
    function updateCustomDropdown(select) {
        if (select._customButton) {
            select._customButton.textContent = select.options[select.selectedIndex]?.text || '';
            if (select._customMenu) {
                select._customMenu.querySelectorAll('.custom-dropdown-item').forEach((item, index) => {
                    item.classList.toggle('selected', index === select.selectedIndex);
                });
            }
        }
    }

    // Destroy custom dropdowns (restore native selects)
    function destroyCustomDropdowns() {
        document.querySelectorAll('.custom-dropdown').forEach(wrapper => {
            const select = wrapper.querySelector('select.form-select');
            if (select) {
                select.style.display = '';
                wrapper.parentNode.insertBefore(select, wrapper);
                delete select._customButton;
                delete select._customMenu;
            }
            wrapper.remove();
        });
    }

    // Device mode modal
    let deviceModeModalOpen = false;

    function showDeviceModeModal() {
        deviceModeModalOpen = true;
        elements.deviceModeModal.classList.add('visible');
        // Highlight current mode
        const currentMode = deviceModeOverride || 'auto';
        elements.deviceModeList.querySelectorAll('.menu-item').forEach(item => {
            item.classList.toggle('selected', item.dataset.mode === currentMode);
        });
    }

    function hideDeviceModeModal() {
        deviceModeModalOpen = false;
        elements.deviceModeModal.classList.remove('visible');
    }

    function applyDeviceMode(mode) {
        hideDeviceModeModal();

        // Set override (null for auto)
        deviceModeOverride = mode === 'auto' ? null : mode;

        // Destroy existing custom dropdowns
        destroyCustomDropdowns();

        // Re-detect device type with new override
        const device = detectDeviceType();
        setFontSize(device.fontSize);
        setupToolbars(device.mode);

        // Re-init custom dropdowns if mobile mode
        if (device.mode === 'phone' || device.mode === 'tablet') {
            initCustomDropdowns();
        }

        // Show confirmation
        appendClientLine('Device mode set to: ' + (mode === 'auto' ? 'Auto (' + device.device + ')' : mode));
    }

    // Setup layout based on device mode
    function setupToolbars(mode) {
        deviceMode = mode;
        // Remove all device classes
        document.body.classList.remove('device-desktop', 'device-tablet', 'device-phone', 'is-mobile');
        // Add the appropriate device class
        document.body.classList.add('device-' + mode);
        // Add is-mobile class for mobile-specific behaviors
        if (mode === 'phone' || mode === 'tablet') {
            document.body.classList.add('is-mobile');
        }
    }

    // Initialize
    function init() {
        // Capture Ctrl+W at window level to prevent browser from closing tab
        // Uses capture phase (true) to intercept before any other handlers
        window.addEventListener('keydown', function(e) {
            if (e.key === 'w' && e.ctrlKey && !e.altKey && !e.metaKey) {
                e.preventDefault();
                e.stopPropagation();
                // Perform word-delete if input is focused (uses kill ring)
                if (document.activeElement === elements.input) {
                    deleteWordBackwardKill();
                } else {
                    // Focus input if not already focused
                    elements.input.focus();
                }
            }
        }, true);  // true = capture phase

        // Detect device type and configure UI
        const device = detectDeviceType();
        setFontSize(device.fontSize);
        setupToolbars(device.mode);

        // Create custom dropdowns on mobile
        if (device.mode === 'phone' || device.mode === 'tablet') {
            initCustomDropdowns();
        }

        setupEventListeners();
        updateAndroidUI();
        loadAuthKey();  // Load saved auth key for passwordless login
        applyTransparency(guiTransparency);  // Set initial #app background in webview mode
        updateTime();
        setInterval(updateTime, 1000);
        // First run: if Android with no host configured, open server settings instead of connecting
        if (window.Android && typeof window.Android.getConnectionInfo === 'function') {
            try {
                var connInfo = JSON.parse(window.Android.getConnectionInfo());
                if (!connInfo.localHost) {
                    openSettingsPopup('clay-server');
                    return;
                }
            } catch(e) {}
        }
        connect();
    }

    // Load auth key from storage (Android only)
    function loadAuthKey() {
        if (window.Android && window.Android.getAuthKey) {
            authKey = window.Android.getAuthKey();
        }
        debugLog('loadAuthKey: ' + (authKey ? 'found key' : 'no key'));
    }

    // Save auth key to storage (Android only)
    function saveAuthKey(key) {
        if (!window.Android) return;
        authKey = key;
        if (window.Android.saveAuthKey) {
            window.Android.saveAuthKey(key);
        }
        debugLog('saveAuthKey: saved key');
    }

    // Clear auth key from storage (Android only)
    function clearAuthKey() {
        authKey = null;
        if (window.Android && window.Android.clearAuthKey) {
            window.Android.clearAuthKey();
        }
        debugLog('clearAuthKey: cleared');
    }

    // Get visible line count in output area
    function getVisibleLineCount() {
        const fontSize = currentFontSize || 14;
        const lineHeight = fontSize * 1.2; // font-size * line-height
        return Math.floor(elements.outputContainer.clientHeight / lineHeight);
    }

    // Get visible column count in output area (approximate from container width and font size)
    function getVisibleColumnCount() {
        const fontSize = currentFontSize || 14;
        const charWidth = fontSize * 0.6; // monospace approximate
        return Math.floor(elements.outputContainer.clientWidth / charWidth);
    }

    // Send UpdateViewState to server for synchronized more-mode
    function sendViewStateIfChanged() {
        const visibleLines = getVisibleLineCount();
        const visibleColumns = getVisibleColumnCount();
        const newState = { worldIndex: currentWorldIndex, visibleLines, visibleColumns };
        if (!lastSentViewState ||
            lastSentViewState.worldIndex !== newState.worldIndex ||
            lastSentViewState.visibleLines !== newState.visibleLines ||
            lastSentViewState.visibleColumns !== newState.visibleColumns) {
            send({
                type: 'UpdateViewState',
                world_index: currentWorldIndex,
                visible_lines: visibleLines,
                visible_columns: visibleColumns
            });
            lastSentViewState = newState;
        }
    }

    // Check if scrolled to bottom
    function isAtBottom() {
        const container = elements.outputContainer;
        return container.scrollHeight - container.scrollTop <= container.clientHeight + 5;
    }

    // Connect to WebSocket server
    let connectionTimeout = null;
    let wakePongTimeout = null;  // Timeout for wake-from-background health check

    // Track if we should try ws:// fallback (for self-signed cert issues)
    let triedWsFallback = false;
    let usingWsFallback = false;

    // Track if we're using native Android WebSocket
    let usingNativeWebSocket = false;
    // Track if we should use native WebSocket (only after browser WebSocket fails)
    let useNativeWebSocket = false;

    // Track alternate host for Android advanced mode (local/remote fallback)
    let alternateHost = null;
    let triedAlternateHost = false;

    // Debug logging - console only (no Toast)
    function debugLog(msg) {
        console.log('[Clay Debug] ' + msg);
    }

    // Check if native Android WebSocket is available
    function hasNativeWebSocket() {
        try {
            return window.Android &&
                   typeof window.Android.hasNativeWebSocket === 'function' &&
                   window.Android.hasNativeWebSocket();
        } catch (e) {
            return false;
        }
    }

    // Set up native WebSocket callbacks (called once)
    function setupNativeWebSocketCallbacks() {
        window.onNativeWebSocketOpen = function() {
            debugLog('Native WS OPEN');
            if (connectionTimeout) {
                clearTimeout(connectionTimeout);
                connectionTimeout = null;
            }
            if (ws) ws.readyState = WebSocket.OPEN;
            connectionFailures = 0;
            triedWsFallback = false;
            triedAlternateHost = false;
            hideCertWarning();
            showConnecting(false);

            // Check for saved credentials (Android auto-login)
            let savedPassword = null;
            let savedUsername = null;
            try {
                if (window.Android && typeof window.Android.getSavedPassword === 'function') {
                    savedPassword = window.Android.getSavedPassword();
                    if (typeof savedPassword !== 'string' || savedPassword.trim() === '') {
                        savedPassword = null;
                    }
                }
                if (window.Android && typeof window.Android.getSavedUsername === 'function') {
                    savedUsername = window.Android.getSavedUsername();
                    if (typeof savedUsername !== 'string' || savedUsername.trim() === '') {
                        savedUsername = null;
                    }
                }
            } catch (e) {
                console.error('Error getting saved credentials:', e);
                savedPassword = null;
                savedUsername = null;
            }

            if (savedPassword) {
                // If we have both username and password, authenticate immediately
                // (user clearly expects multiuser mode)
                if (savedUsername) {
                    enableMultiuserAuthUI();
                    if (elements.authUsername) {
                        elements.authUsername.value = savedUsername;
                    }
                    authenticate(savedPassword, savedUsername);
                } else {
                    // Only password saved - defer until ServerHello tells us if username is needed
                    deferredAutoLoginPassword = savedPassword;
                    deferredAutoLoginUsername = null;
                    // Set a timeout in case ServerHello doesn't arrive
                    setTimeout(function() {
                        if (deferredAutoLoginPassword) {
                            // ServerHello didn't arrive, try auth without username
                            const pwd = deferredAutoLoginPassword;
                            deferredAutoLoginPassword = null;
                            authenticate(pwd, null);
                        }
                    }, 1000);
                }
            } else if (authKey && !keyAuthFailed) {
                // Have auth key but no saved password - wait for ServerHello to try key auth
                // Don't show auth modal yet; it will be shown if key auth fails
                debugLog('onopen: no password but have auth key, waiting for ServerHello');
                // Safety timeout: if ServerHello doesn't arrive within 3s, show auth modal
                setTimeout(function() {
                    if (!authenticated && !authKeyPending) {
                        debugLog('ServerHello timeout, showing auth modal');
                        showAuthModal(true);
                        elements.authPassword.focus();
                    }
                }, 3000);
            } else {
                showAuthModal(true);
                // Pre-fill username if saved (for multiuser mode)
                if (savedUsername && elements.authUsername) {
                    enableMultiuserAuthUI();
                    elements.authUsername.value = savedUsername;
                    elements.authPassword.focus();
                } else {
                    elements.authPassword.focus();
                }
            }
        };

        window.onNativeWebSocketMessage = function(data) {
            try {
                const msg = JSON.parse(data);
                handleMessage(msg);
            } catch (e) {
                console.error('Failed to parse message:', e);
            }
        };

        // Base64 encoded version for safer message passing from Java
        window.onNativeWebSocketMessageBase64 = function(base64Data) {
            try {
                // Decode Base64 to string
                const data = atob(base64Data);
                // Convert from UTF-8 bytes to string
                const bytes = new Uint8Array(data.length);
                for (let i = 0; i < data.length; i++) {
                    bytes[i] = data.charCodeAt(i);
                }
                const decoded = new TextDecoder('utf-8').decode(bytes);
                const msg = JSON.parse(decoded);
                handleMessage(msg);
            } catch (e) {
                console.error('Failed to parse Base64 message:', e);
            }
        };

        window.onNativeWebSocketClose = function(code, reason) {
            debugLog('Native WS CLOSE: ' + code + ' ' + reason);
            if (connectionTimeout) {
                clearTimeout(connectionTimeout);
                connectionTimeout = null;
            }
            if (wakePongTimeout) {
                clearTimeout(wakePongTimeout);
                wakePongTimeout = null;
            }
            if (ws) ws.readyState = WebSocket.CLOSED;
            authenticated = false;
            hasReceivedInitialState = false;

            // Server reload - auto-reconnect with retry
            if (reloadReconnect) {
                reloadReconnectAttempts++;
                if (reloadReconnectAttempts <= 5) {
                    var delay = reloadReconnectAttempts === 1 ? 2000 : 1000;
                    setTimeout(connect, delay);
                } else {
                    reloadReconnect = false;
                }
                return;
            }

            showConnecting(false);
            connectionFailures++;

            if (window.Android && window.Android.stopBackgroundService) {
                window.Android.stopBackgroundService();
            }

            // Try alternate host (Android advanced mode) before giving up
            if (connectionFailures >= 2 && !triedAlternateHost) {
                triedAlternateHost = true;
                if (window.Android && typeof window.Android.getConnectionInfo === 'function') {
                    try {
                        const info = JSON.parse(window.Android.getConnectionInfo());
                        if (info.advancedEnabled && info.remoteHost) {
                            const currentHost = alternateHost || window.WS_HOST || window.location.hostname;
                            const altHost = (currentHost === info.remoteHost) ? info.localHost : info.remoteHost;
                            if (altHost && altHost !== currentHost) {
                                console.log('Native WS failed on ' + currentHost + ', trying alternate: ' + altHost);
                                alternateHost = altHost;
                                connectionFailures = 0;
                                setTimeout(connect, 500);
                                return;
                            }
                        }
                    } catch (e) {
                        console.log('getConnectionInfo error: ' + e);
                    }
                }
            }

            // WebView mode: allow more retries (server may still be starting after reload)
            const maxFailures = window.WEBVIEW_MODE ? 5 : 2;
            if (connectionFailures >= maxFailures) {
                showConnectionErrorModal();
            } else {
                setTimeout(connect, 2000);
            }
        };

        window.onNativeWebSocketError = function(error) {
            debugLog('Native WS ERROR: ' + error);
            showConnecting(false);
        };
    }

    // Initialize native WebSocket callbacks
    if (hasNativeWebSocket()) {
        setupNativeWebSocketCallbacks();

        // Clean up native WebSocket on page unload/reload
        window.addEventListener('beforeunload', function() {
            if (window.Android && window.Android.closeWebSocket) {
                window.Android.closeWebSocket();
            }
        });

        // Also handle pagehide for mobile browsers
        window.addEventListener('pagehide', function() {
            if (window.Android && window.Android.closeWebSocket) {
                window.Android.closeWebSocket();
            }
        });
    }

    function connectWithNativeWebSocket(wsUrl) {
        debugLog('Native WS connecting: ' + wsUrl);
        usingNativeWebSocket = true;

        // Close any existing native WebSocket first
        if (window.Android && window.Android.closeWebSocket) {
            try {
                window.Android.closeWebSocket();
            } catch (e) {
                debugLog('Error closing prev WS: ' + e);
            }
        }

        // Create a fake WebSocket object that bridges to native
        ws = {
            readyState: WebSocket.CONNECTING,
            send: function(data) {
                if (window.Android && window.Android.sendWebSocketMessage) {
                    window.Android.sendWebSocketMessage(data);
                }
            },
            close: function() {
                if (window.Android && window.Android.closeWebSocket) {
                    window.Android.closeWebSocket();
                }
                this.readyState = WebSocket.CLOSED;
            }
        };

        // Set a 5-second timeout for connection
        connectionTimeout = setTimeout(function() {
            if (ws && ws.readyState === WebSocket.CONNECTING) {
                console.log('Native WebSocket connection timeout');
                if (window.Android && window.Android.closeWebSocket) {
                    window.Android.closeWebSocket();
                }
                ws.readyState = WebSocket.CLOSED;
                window.onNativeWebSocketClose(1006, 'Connection timeout');
            }
        }, 5000);

        // Initiate native WebSocket connection
        window.Android.connectWebSocket(wsUrl);
    }

    // Cleanly close any existing socket, nulling its handlers first to prevent stale
    // onclose callbacks from corrupting reconnect state after an explicit close.
    // Resets all fallback flags so we always start fresh on the primary host.
    function forceReconnect() {
        if (wakePongTimeout) { clearTimeout(wakePongTimeout); wakePongTimeout = null; }
        if (connectionTimeout) { clearTimeout(connectionTimeout); connectionTimeout = null; }
        if (ws) {
            ws.onclose = null;
            ws.onerror = null;
            ws.onopen = null;
            ws.onmessage = null;
            try { ws.close(); } catch (e) {}
            ws = null;
        }
        connectionFailures = 0;
        triedWsFallback = false;
        usingWsFallback = false;
        useNativeWebSocket = false;
        usingNativeWebSocket = false;
        alternateHost = null;
        triedAlternateHost = false;
        keyAuthFailed = false;  // Explicit reconnect resets this so key auth can be tried again
        connect();
    }

    function connect() {
        showConnecting(true);

        // Use alternate host if we're in fallback mode, otherwise use WS_HOST
        const host = alternateHost || window.WS_HOST || window.location.hostname;
        // Use ws:// fallback if we've already failed with wss://
        const protocol = usingWsFallback ? 'ws' : window.WS_PROTOCOL;
        // WS_PORT=0 means WS shares the HTTP port (unified server)
        const port = (window.WS_PORT && window.WS_PORT !== 0)
            ? window.WS_PORT : (window.location.port || (protocol === 'wss' ? '443' : '80'));
        const wsUrl = `${protocol}://${host}:${port}`;

        debugLog('connect() protocol=' + protocol + ' hasNative=' + hasNativeWebSocket());

        // Clear any existing timeout
        if (connectionTimeout) {
            clearTimeout(connectionTimeout);
            connectionTimeout = null;
        }

        // If we've determined we need native WebSocket, use it directly
        if (useNativeWebSocket && hasNativeWebSocket()) {
            debugLog('Using native WS (fallback mode)');
            connectWithNativeWebSocket(wsUrl);
            return;
        }

        // On Android with wss://, use native WebSocket directly (handles self-signed certs)
        if (hasNativeWebSocket() && protocol === 'wss') {
            debugLog('Using native WS for wss://');
            connectWithNativeWebSocket(wsUrl);
            return;
        }

        // Standard browser WebSocket
        debugLog('Using browser WebSocket');
        usingNativeWebSocket = false;
        try {
            ws = new WebSocket(wsUrl);

            // Set a 5-second timeout for connection
            connectionTimeout = setTimeout(function() {
                if (ws && ws.readyState === WebSocket.CONNECTING) {
                    ws.close();
                }
            }, 5000);

            ws.onopen = function() {
                if (connectionTimeout) {
                    clearTimeout(connectionTimeout);
                    connectionTimeout = null;
                }
                connectionFailures = 0;
                triedWsFallback = false; // Reset for future reconnects
                triedAlternateHost = false;
                hideCertWarning();
                showConnecting(false);

                // Auto-authenticate for embedded WebView GUI (pre-hashed password)
                if (window.AUTO_PASSWORD) {
                    ws.send(JSON.stringify({ type: 'AuthRequest', password_hash: window.AUTO_PASSWORD, request_key: false }));
                    return;
                }

                // Check for saved credentials (Android auto-login)
                let savedPassword = null;
                let savedUsername = null;
                try {
                    if (window.Android && typeof window.Android.getSavedPassword === 'function') {
                        savedPassword = window.Android.getSavedPassword();
                        // Ensure it's a non-empty string
                        if (typeof savedPassword !== 'string' || savedPassword.trim() === '') {
                            savedPassword = null;
                        }
                    }
                    if (window.Android && typeof window.Android.getSavedUsername === 'function') {
                        savedUsername = window.Android.getSavedUsername();
                        if (typeof savedUsername !== 'string' || savedUsername.trim() === '') {
                            savedUsername = null;
                        }
                    }
                } catch (e) {
                    console.error('Error getting saved credentials:', e);
                    savedPassword = null;
                    savedUsername = null;
                }

                if (savedPassword) {
                    // If we have both username and password, authenticate immediately
                    // (user clearly expects multiuser mode)
                    if (savedUsername) {
                        enableMultiuserAuthUI();
                        if (elements.authUsername) {
                            elements.authUsername.value = savedUsername;
                        }
                        authenticate(savedPassword, savedUsername);
                    } else {
                        // Only password saved - defer until ServerHello tells us if username is needed
                        deferredAutoLoginPassword = savedPassword;
                        deferredAutoLoginUsername = null;
                        // Set a timeout in case ServerHello doesn't arrive
                        setTimeout(function() {
                            if (deferredAutoLoginPassword) {
                                // ServerHello didn't arrive, try auth without username
                                const pwd = deferredAutoLoginPassword;
                                deferredAutoLoginPassword = null;
                                authenticate(pwd, null);
                            }
                        }, 1000);
                    }
                } else if (authKey && !keyAuthFailed) {
                    // Have auth key but no saved password - wait for ServerHello to try key auth
                    debugLog('ws.onopen: no password but have auth key, waiting for ServerHello');
                    // Safety timeout: if ServerHello doesn't arrive within 3s, show auth modal
                    setTimeout(function() {
                        if (!authenticated && !authKeyPending) {
                            debugLog('ServerHello timeout, showing auth modal');
                            showAuthModal(true);
                            elements.authPassword.focus();
                        }
                    }, 3000);
                } else {
                    showAuthModal(true);
                    // Pre-fill username if saved (for multiuser mode)
                    if (savedUsername && elements.authUsername) {
                        enableMultiuserAuthUI();
                        elements.authUsername.value = savedUsername;
                        elements.authPassword.focus();
                    } else {
                        elements.authPassword.focus();
                    }
                }
            };

            ws.onclose = function() {
                if (connectionTimeout) {
                    clearTimeout(connectionTimeout);
                    connectionTimeout = null;
                }
                if (wakePongTimeout) {
                    clearTimeout(wakePongTimeout);
                    wakePongTimeout = null;
                }
                authenticated = false;
                hasReceivedInitialState = false;  // Reset so we use server's world on reconnect

                // Server reload - auto-reconnect with retry
                if (reloadReconnect) {
                    reloadReconnectAttempts++;
                    if (reloadReconnectAttempts <= 5) {
                        var delay = reloadReconnectAttempts === 1 ? 2000 : 1000;
                        setTimeout(connect, delay);
                    } else {
                        reloadReconnect = false;
                    }
                    return;
                }

                showConnecting(false);
                connectionFailures++;
                // Stop Android foreground service when disconnected
                if (window.Android && window.Android.stopBackgroundService) {
                    window.Android.stopBackgroundService();
                }

                // If wss:// failed, try native WebSocket (Android) or ws:// fallback
                if (window.WS_PROTOCOL === 'wss' && !triedWsFallback && !usingWsFallback) {
                    // On Android with native WebSocket, try that first (handles self-signed certs)
                    if (hasNativeWebSocket() && !useNativeWebSocket) {
                        console.log('wss:// connection failed, trying native Android WebSocket...');
                        useNativeWebSocket = true;
                        connectionFailures = 0;
                        setTimeout(connect, 500);
                        return;
                    }
                    // Otherwise try ws:// fallback
                    console.log('wss:// connection failed, trying ws:// fallback...');
                    triedWsFallback = true;
                    usingWsFallback = true;
                    connectionFailures = 0; // Reset failures for fallback attempt
                    setTimeout(connect, 500);
                    return;
                }

                // After 2 failures, try alternate host (Android advanced mode) before giving up
                if (connectionFailures >= 2 && !triedAlternateHost) {
                    triedAlternateHost = true;
                    // Get alternate host from Android bridge
                    if (window.Android && typeof window.Android.getConnectionInfo === 'function') {
                        try {
                            const info = JSON.parse(window.Android.getConnectionInfo());
                            if (info.advancedEnabled && info.remoteHost) {
                                const currentHost = window.WS_HOST || window.location.hostname;
                                // Switch to whichever host we're NOT currently using
                                const altHost = (currentHost === info.remoteHost) ? info.localHost : info.remoteHost;
                                if (altHost && altHost !== currentHost) {
                                    console.log('Connection failed on ' + currentHost + ', trying alternate: ' + altHost);
                                    alternateHost = altHost;
                                    connectionFailures = 0;
                                    triedWsFallback = false;
                                    usingWsFallback = false;
                                    useNativeWebSocket = false;
                                    setTimeout(connect, 500);
                                    return;
                                }
                            }
                        } catch (e) {
                            console.log('getConnectionInfo error: ' + e);
                        }
                    }
                }

                if (connectionFailures >= 2) {
                    // If using wss://, show certificate warning
                    if (window.WS_PROTOCOL === 'wss' && !usingWsFallback && !usingNativeWebSocket) {
                        showCertWarning();
                    }
                    showConnectionErrorModal();
                } else {
                    // First failure - try once more after 3 seconds
                    setTimeout(connect, 3000);
                }
            };

            ws.onerror = function(e) {
                showConnecting(false);
            };

            ws.onmessage = function(event) {
                try {
                    const msg = JSON.parse(event.data);
                    handleMessage(msg);
                } catch (e) {
                    console.error('Failed to parse message:', e);
                }
            };
        } catch (e) {
            showConnecting(false);
            console.error('Failed to connect:', e);
            setTimeout(connect, 3000);
        }
    }

    // Handle incoming messages
    function handleMessage(msg) {
        switch (msg.type) {
            case 'ServerHello':
                // Store challenge for challenge-response auth
                serverChallenge = msg.challenge || '';
                // Server tells us upfront if it's in multiuser mode
                if (msg.multiuser_mode) {
                    enableMultiuserAuthUI();
                }
                // WebView auto-auth already sent from onopen; skip everything else
                if (window.AUTO_PASSWORD) break;
                // Try auth key first (if not multiuser mode - keys are single-user only)
                // Skip if keyAuthFailed: key was rejected this session, go straight to password
                if (!msg.multiuser_mode && authKey && !keyAuthFailed && tryAuthWithKey()) {
                    // Key auth attempt sent, cancel any deferred password auth
                    deferredAutoLoginPassword = null;
                    break;
                }
                // Handle deferred auto-login (Android saved password without username)
                if (deferredAutoLoginPassword) {
                    const pwd = deferredAutoLoginPassword;
                    deferredAutoLoginPassword = null;
                    if (msg.multiuser_mode) {
                        // Server requires username but we don't have one saved
                        // Show auth modal with password pre-filled
                        showAuthModal(true);
                        if (elements.authPassword) {
                            elements.authPassword.value = pwd;
                        }
                        if (elements.authUsername) {
                            elements.authUsername.focus();
                        }
                    } else {
                        // Not multiuser mode - authenticate with just password
                        authenticate(pwd, null);
                    }
                }
                break;

            case 'AuthResponse':
                if (msg.success) {
                    authenticated = true;
                    authKeyPending = false;  // Clear key-based auth flag
                    keyAuthFailed = false;   // Reset so key auth works on next fresh connect
                    reloadReconnect = false;
                    reloadReconnectAttempts = 0;
                    connectionFailures = 0;
                    multiuserMode = msg.multiuser_mode || false;
                    showAuthModal(false);
                    hideConnectionErrorModal();
                    hideReconnectModal();
                    elements.authError.textContent = '';
                    elements.input.focus();
                    // Update UI based on multiuser mode
                    updateMultiuserUI();
                    // Declare client type to server (Web for browser clients)
                    send({ type: 'ClientTypeDeclaration', client_type: 'Web' });
                    // Save password and username for Android auto-login on Activity recreation
                    if (window.Android && window.Android.savePassword && pendingAuthPassword) {
                        window.Android.savePassword(pendingAuthPassword);
                    }
                    if (window.Android && window.Android.saveUsername && pendingAuthUsername) {
                        window.Android.saveUsername(pendingAuthUsername);
                    }
                    pendingAuthPassword = null;
                    pendingAuthUsername = null;
                    // Start Android foreground service to keep connection alive
                    if (window.Android && window.Android.startBackgroundService) {
                        window.Android.startBackgroundService();
                    }
                } else {
                    // If this was a key-based auth failure, show password prompt with key visible
                    if (authKeyPending) {
                        debugLog('Key-based auth failed, showing password prompt with failed key');
                        authKeyPending = false;
                        keyAuthFailed = true;  // Prevent key retry on any auto-reconnect
                        // Don't clear the key - show it in the UI so user can see it failed
                        showAuthModal(true);
                        elements.authError.textContent = 'Auth key rejected - enter password';
                        elements.authPassword.focus();
                        break;
                    }
                    elements.authError.textContent = msg.error || 'Authentication failed';
                    elements.authPassword.value = '';
                    pendingAuthPassword = null;
                    pendingAuthUsername = null;
                    // Clear saved credentials on auth failure (they may be outdated)
                    if (window.Android && window.Android.clearSavedPassword) {
                        window.Android.clearSavedPassword();
                    }
                    if (window.Android && window.Android.clearSavedUsername) {
                        window.Android.clearSavedUsername();
                    }
                    // Detect multiuser mode from error messages
                    if (msg.error === 'Username required' || msg.multiuser_mode) {
                        enableMultiuserAuthUI();
                    }
                    // Show auth modal (may have been hidden during auto-login attempt)
                    showAuthModal(true);
                    if (multiuserMode && elements.authUsername) {
                        elements.authUsername.focus();
                    } else {
                        elements.authPassword.focus();
                    }
                }
                break;

            case 'KeyGenerated':
                // Server sent us a new auth key after successful password auth or regeneration
                if (msg.auth_key) {
                    debugLog('Received auth key from server');
                    saveAuthKey(msg.auth_key);
                    serverAuthKey = msg.auth_key;
                    // Update the web settings input if it's visible
                    if (elements.webAuthKey) {
                        elements.webAuthKey.value = msg.auth_key;
                    }
                }
                break;

            case 'PasswordChanged':
                if (msg.success) {
                    showPasswordModal(false);
                    // Show brief success message in output
                    appendClientLine('Password changed successfully.', currentWorldIndex, 'system');
                } else {
                    elements.passwordError.textContent = msg.error || 'Password change failed';
                }
                break;

            case 'LoggedOut':
                // Server confirmed logout - reset state and show login screen
                worlds = [];
                currentWorldIndex = 0;
                actions = [];
                splashLines = [];
                authenticated = false;
                hasReceivedInitialState = false;
                // Clear output display
                if (elements.output) {
                    elements.output.innerHTML = '';
                }
                // Update status bar to show no world
                updateStatusBar();
                // Show auth modal again
                showAuthModal(true);
                break;

            case 'InitialState':
                worlds = msg.worlds || [];
                // On first connection, use server's world index. On resync, preserve local world.
                if (!hasReceivedInitialState) {
                    currentWorldIndex = msg.current_world_index !== undefined ? msg.current_world_index : 0;
                    hasReceivedInitialState = true;
                } else {
                    // Resync - preserve current world, but validate it's still valid
                    if (currentWorldIndex >= worlds.length) {
                        currentWorldIndex = Math.max(0, worlds.length - 1);
                    }
                }
                actions = msg.actions || [];
                splashLines = msg.splash_lines || [];
                // Reset client-side more-mode state (each client handles more locally)
                paused = false;
                pendingLines = [];
                linesSincePause = 0;
                partialLines = {};
                // Initialize output cache for each world (empty - will be populated on render)
                worldOutputCache = worlds.map(() => []);
                // Ensure output_lines arrays exist, prefer timestamped versions
                const currentTs = Math.floor(Date.now() / 1000);
                worlds.forEach((world) => {
                    // Use output_lines_ts if available (has timestamps)
                    if (world.output_lines_ts && world.output_lines_ts.length > 0) {
                        world.output_lines = world.output_lines_ts;
                    } else if (world.output_lines) {
                        // Convert plain strings to objects with current timestamp
                        world.output_lines = world.output_lines.map(line =>
                            typeof line === 'string' ? { text: line, ts: currentTs } : line
                        );
                    } else {
                        world.output_lines = [];
                    }
                    // Track oldest seq for backfill deduplication
                    world._oldest_seq = null;
                    if (world.output_lines.length > 0) {
                        let minSeq = Infinity;
                        for (const line of world.output_lines) {
                            if (line.seq !== undefined && line.seq < minSeq) minSeq = line.seq;
                        }
                        if (minSeq !== Infinity) world._oldest_seq = minSeq;
                    }
                    // Track max seq for duplicate detection
                    world._max_seq = 0;
                    for (const line of world.output_lines) {
                        if (line.seq !== undefined && line.seq > world._max_seq) {
                            world._max_seq = line.seq;
                        }
                    }
                    // Initialize pending_count from server (for More indicator)
                    if (world.pending_count === undefined) world.pending_count = 0;
                    // Don't merge pending_lines - they stay on the server and are
                    // released via PgDn/Tab, then broadcast as ServerData.
                    // This avoids duplicate lines when pending is released.
                    // Use server's centralized unseen tracking - don't reset to 0
                    // world.unseen_lines comes from server, keep it as-is
                });
                if (msg.settings) {
                    if (msg.settings.input_height) {
                        setInputHeight(msg.settings.input_height);
                    }
                    if (msg.settings.more_mode_enabled !== undefined) {
                        moreModeEnabled = msg.settings.more_mode_enabled;
                    }
                    if (msg.settings.show_tags !== undefined) {
                        showTags = msg.settings.show_tags;
                    }
                    if (msg.settings.ansi_music_enabled !== undefined) {
                        ansiMusicEnabled = msg.settings.ansi_music_enabled;
                    }
                    if (msg.settings.zwj_enabled !== undefined) {
                        zwjEnabled = msg.settings.zwj_enabled;
                    }
                    if (msg.settings.tts_mode !== undefined) ttsMode = msg.settings.tts_mode;
                    if (msg.settings.tts_speak_mode !== undefined) ttsSpeakMode = msg.settings.tts_speak_mode;
                    if (msg.settings.new_line_indicator !== undefined) {
                        newLineIndicator = msg.settings.new_line_indicator;
                    }
                    if (msg.settings.tls_proxy_enabled !== undefined) {
                        tlsProxyEnabled = msg.settings.tls_proxy_enabled;
                    }
                    if (msg.settings.temp_convert_enabled !== undefined) {
                        tempConvertEnabled = msg.settings.temp_convert_enabled;
                    }
                    if (msg.settings.mouse_enabled !== undefined) {
                        mouseEnabled = msg.settings.mouse_enabled;
                    }
                    if (msg.settings.debug_enabled !== undefined) {
                        debugEnabled = msg.settings.debug_enabled;
                    }
                    if (msg.settings.dictionary_path !== undefined) {
                        dictionaryPath = msg.settings.dictionary_path;
                    }
                    if (msg.settings.spell_check_enabled !== undefined) {
                        spellCheckEnabled = msg.settings.spell_check_enabled;
                    }
                    // Web settings
                    if (msg.settings.web_secure !== undefined) {
                        webSecure = msg.settings.web_secure;
                    }
                    if (msg.settings.http_enabled !== undefined) {
                        httpEnabled = msg.settings.http_enabled;
                    }
                    if (msg.settings.http_port !== undefined) {
                        httpPort = msg.settings.http_port;
                    }
                    if (msg.settings.ws_enabled !== undefined) {
                        wsEnabled = msg.settings.ws_enabled;
                    }
                    if (msg.settings.ws_port !== undefined) {
                        wsPort = msg.settings.ws_port;
                    }
                    if (msg.settings.ws_allow_list !== undefined) {
                        wsAllowList = msg.settings.ws_allow_list;
                    }
                    if (msg.settings.ws_cert_file !== undefined && msg.settings.ws_cert_file) {
                        wsCertFile = msg.settings.ws_cert_file;
                    }
                    if (msg.settings.ws_key_file !== undefined && msg.settings.ws_key_file) {
                        wsKeyFile = msg.settings.ws_key_file;
                    }
                    if (msg.settings.tls_configured !== undefined) {
                        tlsConfigured = msg.settings.tls_configured;
                    }
                    if (msg.settings.auth_key !== undefined) {
                        serverAuthKey = msg.settings.auth_key;
                    }
                    if (msg.settings.ws_password !== undefined) {
                        wsPassword = msg.settings.ws_password;
                    }
                    if (msg.settings.world_switch_mode !== undefined) {
                        worldSwitchMode = msg.settings.world_switch_mode;
                    }
                    if (msg.settings.console_theme !== undefined) {
                        consoleTheme = msg.settings.console_theme;
                    }
                    if (msg.settings.gui_theme !== undefined) {
                        guiTheme = msg.settings.gui_theme;
                        applyTheme(guiTheme);
                    }
                    if (msg.settings.theme_colors_json) {
                        applyThemeColors(msg.settings.theme_colors_json);
                        if (window.Android && window.Android.saveThemeCss) {
                            const el = document.getElementById('theme-vars');
                            if (el) window.Android.saveThemeCss(el.textContent);
                        }
                    }
                    if (msg.settings.color_offset_percent !== undefined) {
                        colorOffsetPercent = msg.settings.color_offset_percent;
                    }
                    if (msg.settings.gui_transparency !== undefined) {
                        applyTransparency(msg.settings.gui_transparency);
                    }
                    // Load font name and GUI font size
                    if (msg.settings.font_name !== undefined) {
                        applyFontFamily(msg.settings.font_name);
                    }
                    if (msg.settings.font_size !== undefined) {
                        guiFontSize = msg.settings.font_size;
                    }
                    // Load per-device font sizes
                    if (msg.settings.web_font_size_phone !== undefined) {
                        webFontSizePhone = msg.settings.web_font_size_phone;
                    }
                    if (msg.settings.web_font_size_tablet !== undefined) {
                        webFontSizeTablet = msg.settings.web_font_size_tablet;
                    }
                    if (msg.settings.web_font_size_desktop !== undefined) {
                        webFontSizeDesktop = msg.settings.web_font_size_desktop;
                    }
                    // Pick the right font size based on current device type
                    const fontPx = deviceType === 'phone' ? webFontSizePhone :
                                   deviceType === 'tablet' ? webFontSizeTablet : webFontSizeDesktop;
                    setFontSize(clampFontSize(fontPx), false);  // Don't send back to server
                    // Load font weight
                    if (msg.settings.web_font_weight !== undefined) {
                        webFontWeight = msg.settings.web_font_weight;
                        applyFontWeight(webFontWeight);
                    }
                    if (msg.settings.web_font_line_height !== undefined) webFontLineHeight = msg.settings.web_font_line_height;
                    if (msg.settings.web_font_letter_spacing !== undefined) webFontLetterSpacing = msg.settings.web_font_letter_spacing;
                    if (msg.settings.web_font_word_spacing !== undefined) webFontWordSpacing = msg.settings.web_font_word_spacing;
                    applyAdvancedFontSettings();
                    if (msg.settings.keybindings_json) {
                        try { keybindings = JSON.parse(msg.settings.keybindings_json); } catch(e) {}
                    }
                }
                // Calculate activity count from world data (don't wait for ActivityUpdate message)
                serverActivityCount = worlds.filter((w, i) =>
                    i !== currentWorldIndex && (w.unseen_lines > 0 || (w.pending_count || 0) > 0)
                ).length;
                renderOutput();
                updateStatusBar();
                // Send initial view state for synchronized more-mode
                sendViewStateIfChanged();
                // Schedule lazy backfill of remaining scrollback history
                startBackfill();
                // Lock to specific world if URL parameter specified
                if (lockedWorldName && !lockedWorld) {
                    for (var i = 0; i < worlds.length; i++) {
                        if (worlds[i].name === lockedWorldName) {
                            switchWorldLocal(i);
                            lockedWorld = true;
                            document.title = 'Clay - ' + lockedWorldName;
                            break;
                        }
                    }
                }
                // Grep mode: hide UI, filter output (F2 toggles timestamps)
                if (grepMode) {
                    if (elements.statusBar) elements.statusBar.style.display = 'none';
                    if (elements.inputContainer) elements.inputContainer.style.display = 'none';
                    if (elements.navBar) elements.navBar.style.display = 'none';
                    document.title = 'Clay - grep: ' + grepMode.pattern;
                    renderOutput();
                }

                // Handle pending reconnect command (resend after reconnection)
                if (pendingReconnectCommand !== null) {
                    // Switch to the world that was active when the command failed
                    if (pendingReconnectWorldIndex !== null && pendingReconnectWorldIndex !== currentWorldIndex) {
                        if (pendingReconnectWorldIndex >= 0 && pendingReconnectWorldIndex < worlds.length) {
                            currentWorldIndex = pendingReconnectWorldIndex;
                            renderOutput();
                            updateStatusBar();
                        }
                    }
                    // Resend the command
                    send({
                        type: 'SendCommand',
                        world_index: currentWorldIndex,
                        command: pendingReconnectCommand
                    });
                    // Add to history
                    if (pendingReconnectCommand.length > 0) {
                        commandHistory.push(pendingReconnectCommand);
                        if (commandHistory.length > 1000) {
                            commandHistory.shift();
                        }
                    }
                    // Clear pending state
                    pendingReconnectCommand = null;
                    pendingReconnectWorldIndex = null;
                    elements.input.value = '';
                    elements.prompt.textContent = '';
                }
                break;

            case 'ServerData':
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    const world = worlds[msg.world_index];
                    if (!world.output_lines) world.output_lines = [];
                    // Ensure cache exists for this world
                    if (!worldOutputCache[msg.world_index]) {
                        worldOutputCache[msg.world_index] = [];
                    }
                    // Flush flag: clear output buffer atomically before appending new lines
                    // (e.g., splash screen cleared — combined with data to avoid race condition)
                    if (msg.flush) {
                        world.output_lines = [];
                        world.pendingCount = 0;
                        world.showing_splash = false;
                        worldOutputCache[msg.world_index] = [];
                        partialLines[msg.world_index] = '';
                        world._max_seq = 0; // Reset dedup tracking after flush
                        if (msg.world_index === currentWorldIndex) {
                            elements.output.innerHTML = '';
                            linesSincePause = 0;
                            paused = false;
                            pendingLines = [];
                        }
                    }
                    if (msg.data) {
                        // Dedup: skip ServerData that has already been received (e.g., after resync)
                        if (msg.seq && msg.seq > 0 && world._max_seq && msg.seq <= world._max_seq) {
                            const dupInfo = {
                                world_index: msg.world_index,
                                msg_seq: msg.seq,
                                max_seq: world._max_seq,
                                line_count: msg.data.split('\n').length,
                                first_line: msg.data.substring(0, 200),
                                timestamp: new Date().toISOString()
                            };
                            console.warn('DUPLICATE ServerData detected:', dupInfo);
                            // Report to server for persistent logging
                            send({
                                type: 'ReportDuplicate',
                                world_index: msg.world_index,
                                line_seq: msg.seq,
                                max_seq: world._max_seq,
                                line_text: msg.data.substring(0, 200),
                                source: window.Android ? 'android' : 'web'
                            });
                            break;
                        }

                        // Get timestamp from message or use current time
                        const lineTs = msg.ts || Math.floor(Date.now() / 1000);

                        // Client-generated messages (from_server: false) are always complete
                        // Only use partial line handling for MUD server data
                        const isFromServer = msg.from_server !== false;

                        // Prepend any partial line from previous read (only for server data)
                        let data = msg.data;
                        if (isFromServer && partialLines[msg.world_index]) {
                            data = partialLines[msg.world_index] + data;
                            partialLines[msg.world_index] = '';
                        }

                        // Check if data ends with a newline (complete line)
                        const endsWithNewline = /[\r\n]$/.test(data);

                        // Split by any line ending
                        const rawLines = data.split(/\r\n|\n|\r/);

                        // Remove trailing empty string from split (data ending with \n
                        // produces ["line", ""] — the empty string is not a real line)
                        if (endsWithNewline && rawLines.length > 0 && rawLines[rawLines.length - 1] === '') {
                            rawLines.pop();
                        }

                        // If data doesn't end with newline, last element is a partial line
                        // (only for server data - client messages are always complete)
                        if (isFromServer && !endsWithNewline && rawLines.length > 0) {
                            partialLines[msg.world_index] = rawLines.pop();
                        }

                        let appendedLineCount = 0;
                        rawLines.forEach(line => {
                            // Skip lines that are ONLY ANSI codes with no visible content
                            // (e.g., trailing reset codes after newlines), but keep blank lines
                            if (line.length > 0 && line.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '').length === 0) {
                                return;
                            }
                            // Filter out keep-alive idler message lines
                            if (line.includes('###_idler_message_') && line.includes('_###')) {
                                return;
                            }
                            // Grep mode: skip non-matching lines
                            // Match against displayed text (strip ANSI codes AND MUD tags)
                            if (grepRegex) {
                                const plainLine = stripMudTag(line.replace(/\x1b\[[0-9;]*[A-Za-z]/g, ''));
                                if (!grepRegex.test(plainLine)) {
                                    return;
                                }
                            }
                            const lineIndex = world.output_lines.length;
                            const hasRealSeq = msg.seq !== undefined && msg.seq > 0;
                            const lineSeq = hasRealSeq ? msg.seq + appendedLineCount : lineIndex;
                            world.output_lines.push({ text: truncateIfNeeded(line), ts: lineTs, seq: lineSeq, from_server: isFromServer, _has_real_seq: hasRealSeq, marked_new: msg.marked_new || false, gagged: msg.gagged || false });
                            appendedLineCount++;
                            // Verify sequence order (only for messages with real server-assigned seq)
                            if (lineIndex > 0 && msg.seq !== undefined && msg.seq > 0) {
                                const prevLine = world.output_lines[lineIndex - 1];
                                // Only compare against previous lines that also have real seqs
                                if (prevLine.seq !== undefined && prevLine._has_real_seq && lineSeq <= prevLine.seq) {
                                    console.warn('SEQ MISMATCH in world ' + msg.world_index + ': idx=' + lineIndex + ' expected seq>' + prevLine.seq + ' got seq=' + lineSeq);
                                    send({
                                        type: 'ReportSeqMismatch',
                                        world_index: msg.world_index,
                                        expected_seq_gt: prevLine.seq,
                                        actual_seq: lineSeq,
                                        line_text: line.substring(0, 80),
                                        source: window.Android ? 'android' : 'web'
                                    });
                                }
                            }
                            if (msg.world_index === currentWorldIndex) {
                                const lineMarkedNew = msg.marked_new || false;
                                // Gagged lines are stored but not rendered (only visible with F2)
                                // They bypass more-mode entirely
                                if (msg.gagged) {
                                    // Don't render or count for more-mode
                                } else if (!hasRealSeq && isFromServer) {
                                    // Released pending lines (seq=0, from_server=true) bypass local
                                    // more-mode to avoid flickering the More indicator
                                    appendNewLine(line, lineTs, msg.world_index, lineIndex, lineMarkedNew);
                                } else {
                                    handleIncomingLine(line, lineTs, msg.world_index, lineIndex, lineMarkedNew);
                                }
                            }
                            // Note: Don't track unseen_lines locally - server handles centralized tracking
                            // and sends UnseenUpdate messages to keep all clients in sync
                        });
                        // Update _max_seq after appending lines
                        if (msg.seq && msg.seq > 0 && appendedLineCount > 0) {
                            world._max_seq = Math.max(world._max_seq || 0, msg.seq + appendedLineCount - 1);
                        }
                        if (msg.world_index !== currentWorldIndex) {
                            updateStatusBar();
                        }
                        // After flush, force full re-render to ensure output is visible
                        // (handles case where splash image was re-rendered by WorldConnected)
                        if (msg.flush && msg.world_index === currentWorldIndex) {
                            renderOutput();
                        }
                    }
                }
                break;

            case 'WorldConnected':
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].connected = true;
                    worlds[msg.world_index].was_connected = true;
                    updateStatusBar();
                    // If viewing this world, ensure output is rendered
                    // This handles cases where data arrived before WorldConnected
                    if (msg.world_index === currentWorldIndex) {
                        renderOutput();
                    }
                }
                break;

            case 'WorldDisconnected':
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].connected = false;
                    updateStatusBar();
                }
                break;

            case 'WorldAdded':
                if (msg.world) {
                    const world = msg.world;
                    const currentTs = Math.floor(Date.now() / 1000);
                    // Convert output_lines to timestamped format (same as InitialState)
                    if (world.output_lines_ts && world.output_lines_ts.length > 0) {
                        world.output_lines = world.output_lines_ts;
                    } else if (world.output_lines) {
                        world.output_lines = world.output_lines.map(line =>
                            typeof line === 'string' ? { text: line, ts: currentTs } : line
                        );
                    } else {
                        world.output_lines = [];
                    }
                    // Don't merge pending_lines - they stay on the server
                    // and are released via PgDn/Tab to avoid duplicates
                    // Insert at the correct index
                    const insertIndex = world.index !== undefined ? world.index : worlds.length;
                    worlds.splice(insertIndex, 0, world);
                    // Adjust currentWorldIndex if the new world was inserted before it
                    if (currentWorldIndex >= insertIndex) {
                        currentWorldIndex++;
                    }
                    // Adjust selectedWorldIndex if needed
                    if (selectedWorldIndex >= insertIndex) {
                        selectedWorldIndex++;
                    }
                    // Update output cache array
                    worldOutputCache.splice(insertIndex, 0, []);
                    updateStatusBar();
                    if (worldSelectorPopupOpen) {
                        renderWorldSelectorList();
                    }
                }
                break;

            case 'WorldCreated':
                // Server created a new world at our request - open the editor
                if (msg.world_index !== undefined && msg.world_index < worlds.length) {
                    openWorldEditorPopup(msg.world_index);
                }
                break;

            case 'WorldRemoved':
                if (msg.world_index !== undefined && msg.world_index < worlds.length) {
                    worlds.splice(msg.world_index, 1);
                    // Adjust currentWorldIndex if needed
                    if (currentWorldIndex >= worlds.length) {
                        currentWorldIndex = Math.max(0, worlds.length - 1);
                    } else if (currentWorldIndex > msg.world_index) {
                        currentWorldIndex--;
                    }
                    // Adjust selectedWorldIndex if needed
                    if (selectedWorldIndex >= worlds.length) {
                        selectedWorldIndex = Math.max(0, worlds.length - 1);
                    } else if (selectedWorldIndex > msg.world_index) {
                        selectedWorldIndex--;
                    }
                    updateStatusBar();
                    renderOutput();
                    if (worldSelectorPopupOpen) {
                        renderWorldSelectorList();
                    }
                }
                break;

            case 'WorldSwitched':
                // Console switched worlds - we ignore this to maintain independent view
                // Web interface tracks its own current world separately
                break;

            case 'WorldFlushed':
                // Clear output buffer for this world (splash screen cleared, etc.)
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].output_lines = [];
                    worlds[msg.world_index].pendingCount = 0;
                    worlds[msg.world_index].showing_splash = false;
                    // Clear the cache for this world
                    if (worldOutputCache[msg.world_index]) {
                        worldOutputCache[msg.world_index] = [];
                    }
                    // Clear any partial line buffer
                    partialLines[msg.world_index] = '';
                    // If it's the current world, clear the display and reset more-mode state
                    if (msg.world_index === currentWorldIndex) {
                        elements.output.innerHTML = '';
                        // Reset more-mode state to prevent immediate pause on new data
                        linesSincePause = 0;
                        paused = false;
                        pendingLines = [];
                    }
                }
                break;

            case 'PromptUpdate':
                // Always store the prompt in the world object
                if (msg.world_index >= 0 && msg.world_index < worlds.length) {
                    worlds[msg.world_index].prompt = msg.prompt || '';
                }
                // Update display if it's the current world
                if (msg.world_index === currentWorldIndex) {
                    if (msg.prompt) {
                        elements.prompt.innerHTML = sanitizeHtml(parseAnsi(msg.prompt));
                    } else {
                        elements.prompt.textContent = '';
                    }
                }
                break;

            case 'SetInputBuffer':
                if (msg.text != null) {
                    elements.input.value = msg.text;
                    if (msg.cursor_start) {
                        elements.input.selectionStart = elements.input.selectionEnd = 0;
                    } else {
                        elements.input.selectionStart = elements.input.selectionEnd = msg.text.length;
                    }
                }
                break;

            case 'ThemeCssVarsUpdated':
                // Live theme update from theme editor
                if (msg.css_vars) {
                    var themeVarsEl = document.getElementById('theme-vars');
                    if (themeVarsEl) {
                        themeVarsEl.textContent = ':root { ' + msg.css_vars + ' }';
                    }
                    // Reset cached ANSI palette so it re-reads from CSS vars
                    themeAnsiPalette = null;
                    colorNameToRgb = null;
                    renderOutput();
                }
                break;

            case 'GlobalSettingsUpdated':
                if (msg.settings) {
                    if (msg.settings.input_height) {
                        setInputHeight(msg.settings.input_height);
                    }
                    if (msg.settings.more_mode_enabled !== undefined) {
                        moreModeEnabled = msg.settings.more_mode_enabled;
                    }
                    if (msg.settings.show_tags !== undefined) {
                        const oldShowTags = showTags;
                        showTags = msg.settings.show_tags;
                        if (oldShowTags !== showTags) {
                            renderOutput(); // Re-render with new tag visibility
                        }
                    }
                    if (msg.settings.ansi_music_enabled !== undefined) {
                        ansiMusicEnabled = msg.settings.ansi_music_enabled;
                    }
                    if (msg.settings.zwj_enabled !== undefined) {
                        zwjEnabled = msg.settings.zwj_enabled;
                    }
                    if (msg.settings.tts_mode !== undefined) ttsMode = msg.settings.tts_mode;
                    if (msg.settings.tts_speak_mode !== undefined) ttsSpeakMode = msg.settings.tts_speak_mode;
                    if (msg.settings.new_line_indicator !== undefined) {
                        const oldNli = newLineIndicator;
                        newLineIndicator = msg.settings.new_line_indicator;
                        if (oldNli !== newLineIndicator) {
                            renderOutput();
                        }
                    }
                    if (msg.settings.tls_proxy_enabled !== undefined) {
                        tlsProxyEnabled = msg.settings.tls_proxy_enabled;
                    }
                    if (msg.settings.temp_convert_enabled !== undefined) {
                        tempConvertEnabled = msg.settings.temp_convert_enabled;
                    }
                    if (msg.settings.mouse_enabled !== undefined) {
                        mouseEnabled = msg.settings.mouse_enabled;
                    }
                    if (msg.settings.debug_enabled !== undefined) {
                        debugEnabled = msg.settings.debug_enabled;
                    }
                    if (msg.settings.dictionary_path !== undefined) {
                        dictionaryPath = msg.settings.dictionary_path;
                    }
                    if (msg.settings.spell_check_enabled !== undefined) {
                        spellCheckEnabled = msg.settings.spell_check_enabled;
                    }
                    if (msg.settings.world_switch_mode !== undefined) {
                        worldSwitchMode = msg.settings.world_switch_mode;
                    }
                    // Web settings
                    if (msg.settings.web_secure !== undefined) {
                        webSecure = msg.settings.web_secure;
                    }
                    if (msg.settings.http_enabled !== undefined) {
                        httpEnabled = msg.settings.http_enabled;
                    }
                    if (msg.settings.http_port !== undefined) {
                        httpPort = msg.settings.http_port;
                    }
                    if (msg.settings.ws_enabled !== undefined) {
                        wsEnabled = msg.settings.ws_enabled;
                    }
                    if (msg.settings.ws_port !== undefined) {
                        wsPort = msg.settings.ws_port;
                    }
                    if (msg.settings.ws_allow_list !== undefined) {
                        wsAllowList = msg.settings.ws_allow_list;
                    }
                    if (msg.settings.ws_cert_file !== undefined && msg.settings.ws_cert_file) {
                        wsCertFile = msg.settings.ws_cert_file;
                    }
                    if (msg.settings.ws_key_file !== undefined && msg.settings.ws_key_file) {
                        wsKeyFile = msg.settings.ws_key_file;
                    }
                    if (msg.settings.tls_configured !== undefined) {
                        tlsConfigured = msg.settings.tls_configured;
                    }
                    if (msg.settings.auth_key !== undefined) {
                        serverAuthKey = msg.settings.auth_key;
                    }
                    if (msg.settings.ws_password !== undefined) {
                        wsPassword = msg.settings.ws_password;
                    }
                    if (msg.settings.console_theme !== undefined) {
                        consoleTheme = msg.settings.console_theme;
                    }
                    if (msg.settings.gui_theme !== undefined) {
                        guiTheme = msg.settings.gui_theme;
                        applyTheme(guiTheme);
                    }
                    if (msg.settings.theme_colors_json) {
                        applyThemeColors(msg.settings.theme_colors_json);
                        if (window.Android && window.Android.saveThemeCss) {
                            const el = document.getElementById('theme-vars');
                            if (el) window.Android.saveThemeCss(el.textContent);
                        }
                    }
                    if (msg.settings.color_offset_percent !== undefined) {
                        const oldOffset = colorOffsetPercent;
                        colorOffsetPercent = msg.settings.color_offset_percent;
                        if (oldOffset !== colorOffsetPercent) {
                            renderOutput(); // Re-render with new color offset
                        }
                    }
                    if (msg.settings.gui_transparency !== undefined) {
                        applyTransparency(msg.settings.gui_transparency);
                    }
                    // Font settings
                    if (msg.settings.font_name !== undefined) {
                        applyFontFamily(msg.settings.font_name);
                    }
                    if (msg.settings.font_size !== undefined) {
                        guiFontSize = msg.settings.font_size;
                    }
                    if (msg.settings.web_font_size_phone !== undefined) {
                        webFontSizePhone = msg.settings.web_font_size_phone;
                    }
                    if (msg.settings.web_font_size_tablet !== undefined) {
                        webFontSizeTablet = msg.settings.web_font_size_tablet;
                    }
                    if (msg.settings.web_font_size_desktop !== undefined) {
                        webFontSizeDesktop = msg.settings.web_font_size_desktop;
                    }
                    // Apply the right font size for current device type
                    if (msg.settings.web_font_size_phone !== undefined ||
                        msg.settings.web_font_size_tablet !== undefined ||
                        msg.settings.web_font_size_desktop !== undefined) {
                        const fontPx = deviceType === 'phone' ? webFontSizePhone :
                                       deviceType === 'tablet' ? webFontSizeTablet : webFontSizeDesktop;
                        setFontSize(clampFontSize(fontPx), false);
                    }
                    // Apply font weight
                    if (msg.settings.web_font_weight !== undefined) {
                        webFontWeight = msg.settings.web_font_weight;
                        applyFontWeight(webFontWeight);
                    }
                    if (msg.settings.web_font_line_height !== undefined) webFontLineHeight = msg.settings.web_font_line_height;
                    if (msg.settings.web_font_letter_spacing !== undefined) webFontLetterSpacing = msg.settings.web_font_letter_spacing;
                    if (msg.settings.web_font_word_spacing !== undefined) webFontWordSpacing = msg.settings.web_font_word_spacing;
                    applyAdvancedFontSettings();
                    if (msg.settings.keybindings_json) {
                        try { keybindings = JSON.parse(msg.settings.keybindings_json); } catch(e) {}
                    }
                }
                break;

            case 'KeybindingsUpdated':
                if (msg.bindings_json) {
                    try { keybindings = JSON.parse(msg.bindings_json); } catch(e) {}
                }
                break;

            case 'Pong':
                // Keepalive response - also used for connection health check on wake
                if (wakePongTimeout) {
                    clearTimeout(wakePongTimeout);
                    wakePongTimeout = null;
                    // Connection is alive - no resync needed, just update view state
                    sendViewStateIfChanged();
                }
                break;

            case 'PingCheck':
                // Server liveness check for /remote command - respond immediately
                send({ type: 'PongCheck', nonce: msg.nonce || 0 });
                break;

            case 'ActionsUpdated':
                actions = msg.actions || [];
                if (actionsListPopupOpen) {
                    renderActionsList();
                }
                break;

            case 'CalculatedWorld':
                // Server calculated next/prev world - switch to it
                if (msg.index !== null && msg.index !== undefined && msg.index !== currentWorldIndex) {
                    switchWorldLocal(msg.index);
                }
                break;

            case 'UnseenCleared':
                // Another client (console, web, or GUI) has viewed this world
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].unseen_lines = 0;
                    updateStatusBar();
                }
                break;

            case 'UnseenUpdate':
                // Server's unseen count changed - update our copy
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].unseen_lines = msg.count || 0;
                    updateStatusBar();
                }
                break;

            case 'ActivityUpdate':
                // Server's activity count - just display it
                serverActivityCount = msg.count || 0;
                updateStatusBar();
                break;

            case 'ShowTagsChanged':
                // Server toggled show_tags (F2 or /tag command)
                showTags = msg.show_tags;
                renderOutput();
                break;

            case 'PendingLinesUpdate':
                // Update pending count for a world (used for activity indicator)
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].pending_count = msg.count || 0;
                    updateStatusBar();
                }
                break;

            case 'PendingReleased':
                // Server/another client released pending lines - sync our state
                // Reset linesSincePause because released lines are broadcast as ServerData
                // and would otherwise inflate the counter, causing premature more-mode trigger
                linesSincePause = 0;
                if (msg.world_index === currentWorldIndex && msg.count > 0) {
                    doReleasePending(msg.count);
                }
                break;

            case 'ExecuteLocalCommand':
                // Server wants us to execute a command locally (from action)
                if (msg.command) {
                    executeLocalCommand(msg.command);
                }
                break;

            case 'OpenWindow':
                var worldParam = msg.world ? '?world=' + encodeURIComponent(msg.world) : '';
                // Use WS_HOST/WS_PORT if available (WebView uses custom protocol, not real host)
                var wsProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                var wsHost = window.WS_HOST || window.location.hostname;
                var wsPort = (window.WS_PORT && window.WS_PORT !== 0)
                    ? window.WS_PORT : window.location.port;
                var openUrl = wsProto + '://' + wsHost + ':' + wsPort + '/' + worldParam;
                window.open(openUrl, '_blank');
                break;

            case 'AnsiMusic':
                // Play ANSI music notes via Web Audio API
                if (msg.notes && msg.notes.length > 0) {
                    playAnsiMusic(msg.notes);
                }
                break;

            case 'GmcpData':
                // Store GMCP data for script access
                break;

            case 'MsdpData':
                // Store MSDP data for script access
                break;

            case 'McmpMedia':
                // Handle MCMP media commands (Play/Stop/Load/Default)
                if (msg.action === 'Default') {
                    handleMcmpMedia(msg.action, msg.data, msg.default_url);
                } else if (worlds[msg.world_index] && worlds[msg.world_index].gmcp_user_enabled
                           && msg.world_index === currentWorldIndex) {
                    handleMcmpMedia(msg.action, msg.data, msg.default_url);
                }
                break;

            case 'GmcpUserToggled':
                if (worlds[msg.world_index]) {
                    worlds[msg.world_index].gmcp_user_enabled = msg.enabled;
                    if (!msg.enabled && msg.world_index === currentWorldIndex) {
                        mcmpStopAll();
                    }
                    updateStatusBar();
                }
                break;

            case 'BanListResponse':
                // Ban list received - output is already sent via ServerData
                // This message can be used for future UI enhancements
                break;

            case 'UnbanResult':
                // Unban result received - output is already sent via ServerData
                // This message can be used for future UI enhancements
                break;

            case 'WorldStateResponse':
                // Response to RequestWorldState - update state for the world
                if (msg.world_index === currentWorldIndex) {
                    const world = worlds[msg.world_index];
                    if (world) {
                        // Update pending count
                        world.pending_count = msg.pending_count || 0;
                        // Update prompt
                        world.prompt = msg.prompt || '';
                        if (world.prompt) {
                            elements.prompt.innerHTML = sanitizeHtml(parseAnsi(world.prompt));
                        } else {
                            elements.prompt.textContent = '';
                        }
                        // Update status bar to show more indicator
                        updateStatusBar();
                    }
                }
                break;

            case 'Notification':
                // Send notification to Android app if available
                if (window.Android && window.Android.showNotification) {
                    window.Android.showNotification(msg.title || 'Clay', msg.message || '');
                }
                break;

            case 'ServerSpeak':
                // Text-to-speech via Web Speech API
                if (window.speechSynthesis && msg.text) {
                    var utterance = new SpeechSynthesisUtterance(msg.text);
                    window.speechSynthesis.speak(utterance);
                }
                break;

            case 'WorldSwitchResult':
                // Response to CycleWorld - update local world index and state
                if (msg.world_index !== undefined) {
                    currentWorldIndex = msg.world_index;
                    if (worlds[msg.world_index]) {
                        worlds[msg.world_index].pending_count = msg.pending_count || 0;
                        worlds[msg.world_index].paused = msg.paused || false;
                    }
                    updateStatusBar();
                    renderOutput();
                    // Send MarkWorldSeen since we're now viewing this world
                    send({
                        type: 'MarkWorldSeen',
                        world_index: currentWorldIndex
                    });
                }
                break;

            case 'OutputLines':
                // Batch of output lines from server (initial or incremental)
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    const world = worlds[msg.world_index];
                    const lines = msg.lines || [];
                    for (const line of lines) {
                        world.output_lines.push({
                            text: line.text,
                            ts: line.ts,
                            gagged: line.gagged || false,
                            from_server: line.from_server !== false,
                            seq: line.seq || 0,
                            highlight_color: line.highlight_color,
                            marked_new: line.marked_new || false
                        });
                    }
                    if (msg.world_index === currentWorldIndex) {
                        renderOutput();
                    }
                }
                break;

            case 'PendingCountUpdate':
                // Periodic pending count update from server
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].pending_count = msg.count || 0;
                    updateStatusBar();
                }
                break;

            case 'ScrollbackLines':
                // Response to RequestScrollback - prepend lines to output
                if (msg.world_index !== undefined && worlds[msg.world_index] && msg.lines && msg.lines.length > 0) {
                    const world = worlds[msg.world_index];
                    const wasBottom = isAtBottom();
                    const container = elements.outputContainer;
                    const oldScrollHeight = container.scrollHeight;

                    // Prepend received lines (they are older than what we have)
                    world.output_lines = msg.lines.concat(world.output_lines);

                    // Update oldest seq for next backfill request
                    let minSeq = Infinity;
                    for (const line of msg.lines) {
                        if (line.seq !== undefined && line.seq < minSeq) minSeq = line.seq;
                    }
                    if (minSeq !== Infinity) world._oldest_seq = minSeq;

                    // Re-render only if user has scrolled up into history.
                    // Backfill lines are old content at the top — not visible when at bottom.
                    // Skipping renderOutput() avoids restarting CSS animations (e.g. blink).
                    // In grep mode, always re-render to show newly available matching lines.
                    if (msg.world_index === currentWorldIndex && (!wasBottom || grepRegex)) {
                        renderOutput();
                        // Adjust scrollTop by the height difference (new content at top)
                        {
                            const newScrollHeight = container.scrollHeight;
                            container.scrollTop += (newScrollHeight - oldScrollHeight);
                        }
                    }
                }
                // Continue or finish backfill
                if (msg.backfill_complete) {
                    backfillNextWorld();
                } else {
                    scheduleNextBackfillChunk();
                }
                break;

            case 'ServerReloading':
                reloadReconnect = true;
                reloadReconnectAttempts = 0;
                break;

            default:
                console.log('Unknown message type:', msg.type);
        }
    }

    // Handle incoming line with more-mode logic
    function handleIncomingLine(text, ts, worldIndex, lineIndex, markedNew) {
        if (text === undefined || text === null) return;

        const visibleLines = getVisibleLineCount();
        const threshold = Math.max(1, visibleLines - 2);

        if (paused) {
            // Already paused, queue the line info
            pendingLines.push({ text, ts, worldIndex, lineIndex, markedNew: markedNew || false });
            updateStatusBar();
        } else if (moreModeEnabled && linesSincePause >= threshold) {
            // Trigger pause
            paused = true;
            pendingLines.push({ text, ts, worldIndex, lineIndex, markedNew: markedNew || false });
            // Scroll to bottom to show what we have so far
            scrollToBottom();
            updateStatusBar();
        } else {
            // Normal display - append the line
            linesSincePause++;
            appendNewLine(text, ts, worldIndex, lineIndex, markedNew);
        }
    }

    // Release one screenful of pending lines
    function releaseScreenful() {
        const world = worlds[currentWorldIndex];
        const serverPending = world ? (world.pending_count || 0) : 0;

        // Check if there's anything to release (local or server)
        if (pendingLines.length === 0 && serverPending === 0) return;

        const count = Math.max(1, getVisibleLineCount() - 2);

        // Release local pending lines
        if (pendingLines.length > 0) {
            doReleasePending(count);
        }

        // Also request server to release pending lines
        if (serverPending > 0) {
            // Optimistic UI update: immediately reduce pending_count so rapid PageDown
            // presses don't send redundant requests. Server will correct with PendingLinesUpdate.
            const toRelease = Math.min(count, serverPending);
            world.pending_count = Math.max(0, serverPending - toRelease);
            updateStatusBar();
            send({ type: 'ReleasePending', world_index: currentWorldIndex, count: count });
        }
    }

    // Release all pending lines
    function releaseAll() {
        const world = worlds[currentWorldIndex];
        const serverPending = world ? (world.pending_count || 0) : 0;

        // Release local pending lines
        if (pendingLines.length > 0) {
            doReleasePending(0);
        }

        // Also request server to release all pending lines
        if (serverPending > 0) {
            // Optimistic UI update: immediately set pending_count to 0
            world.pending_count = 0;
            updateStatusBar();
            send({ type: 'ReleasePending', world_index: currentWorldIndex, count: 0 });
        }
    }

    // Actually release pending lines (called when server broadcasts PendingReleased)
    function doReleasePending(count) {
        if (pendingLines.length === 0) return;

        const toRelease = count === 0 ? pendingLines.length : Math.min(count, pendingLines.length);
        const released = pendingLines.splice(0, toRelease);

        released.forEach(item => {
            appendNewLine(item.text, item.ts, item.worldIndex, item.lineIndex, item.markedNew);
        });

        if (pendingLines.length === 0) {
            paused = false;
            linesSincePause = 0;
        }

        updateStatusBar();
    }

    // Send message to server - returns true if sent, false if connection lost
    function send(msg) {
        if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
            ws.send(JSON.stringify(msg));
            return true;
        }
        return false;
    }

    // --- Lazy Backfill Orchestration ---

    // Start backfill after InitialState is processed.
    // Builds a queue of worlds that need backfill (current world first).
    function startBackfill() {
        backfillInProgress = false;
        backfillWorldQueue = [];
        backfillCurrentWorld = null;

        // Build queue: current world first, then others
        const queue = [];
        worlds.forEach((world, idx) => {
            const total = world.total_output_lines || 0;
            const received = world.output_lines ? world.output_lines.length : 0;
            if (total > received && world._oldest_seq !== null) {
                if (idx === currentWorldIndex) {
                    queue.unshift(idx);
                } else {
                    queue.push(idx);
                }
            }
        });

        if (queue.length === 0) return;

        backfillWorldQueue = queue;
        backfillInProgress = true;
        // Delay before first request to let UI settle
        setTimeout(function() {
            backfillNextWorld();
        }, 500);
    }

    // Move to the next world in the backfill queue
    function backfillNextWorld() {
        if (backfillWorldQueue.length === 0) {
            backfillInProgress = false;
            backfillCurrentWorld = null;
            return;
        }
        backfillCurrentWorld = backfillWorldQueue.shift();
        requestBackfillChunk(backfillCurrentWorld);
    }

    // Send a RequestScrollback for the given world
    function requestBackfillChunk(worldIndex) {
        const world = worlds[worldIndex];
        if (!world || world._oldest_seq === null) {
            // Nothing to backfill, skip to next world
            backfillNextWorld();
            return;
        }
        send({
            type: 'RequestScrollback',
            world_index: worldIndex,
            count: BACKFILL_CHUNK_SIZE,
            before_seq: world._oldest_seq
        });
    }

    // Schedule the next chunk after a short delay
    function scheduleNextBackfillChunk() {
        if (backfillCurrentWorld === null) return;
        const worldIdx = backfillCurrentWorld;
        setTimeout(function() {
            if (backfillCurrentWorld === worldIdx) {
                requestBackfillChunk(worldIdx);
            }
        }, BACKFILL_DELAY_MS);
    }

    // Try to authenticate with saved auth key (passwordless)
    async function tryAuthWithKey() {
        if (!authKey || !ws || ws.readyState !== WebSocket.OPEN) return false;

        debugLog('tryAuthWithKey: attempting key-based auth');
        authKeyPending = true;
        // Challenge-response: send SHA256(auth_key + challenge) instead of raw key
        let keyValue = authKey;
        let usesChallenge = false;
        if (serverChallenge) {
            try {
                keyValue = await hashPassword(authKey + serverChallenge);
                usesChallenge = true;
            } catch (e) {
                keyValue = sha256Fallback(authKey + serverChallenge);
                usesChallenge = true;
            }
        }
        const msg = {
            type: 'AuthRequest',
            password_hash: '',  // Empty - using key instead
            auth_key: keyValue,
            challenge_response: usesChallenge,
            request_key: false
        };
        if (currentWorldIndex !== undefined) {
            msg.current_world = currentWorldIndex;
        }
        ws.send(JSON.stringify(msg));
        return true;
    }

    // Authenticate - sends directly via ws.send since authenticated is still false
    // passwordOverride and usernameOverride are used for Android auto-login
    function authenticate(passwordOverride, usernameOverride) {
        // Trim password to remove any trailing spaces from Android keyboard
        const rawPassword = passwordOverride || elements.authPassword.value;
        const password = String(rawPassword || '').trim();

        // Check if user edited the auth key field
        if (elements.authKeyInput && window.Android) {
            const editedKey = elements.authKeyInput.value.trim();
            if (editedKey && editedKey !== authKey) {
                // User changed the auth key - save it
                saveAuthKey(editedKey);
            }
            // If no password but we have an auth key, try key-based auth
            if (!password && authKey) {
                if (ws && ws.readyState === WebSocket.OPEN) {
                    tryAuthWithKey();
                }
                return;
            }
        }

        if (!password) return;
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            // No live connection — defer the password and reconnect.
            // After reconnect ServerHello will use deferredAutoLoginPassword directly,
            // skipping key auth since keyAuthFailed is set.
            deferredAutoLoginPassword = password;
            deferredAutoLoginUsername = usernameOverride ||
                (elements.authUsername && elements.authUsernameRow.style.display !== 'none'
                    ? (elements.authUsername.value.trim() || null)
                    : null);
            keyAuthFailed = true;  // Don't try key auth on this reconnect
            forceReconnect();
            return;
        }

        // Store password for saving on success (Android auto-login)
        pendingAuthPassword = password;

        // Get username: prefer override (auto-login), then UI element if visible
        let username = usernameOverride || null;
        if (!username && elements.authUsername && elements.authUsernameRow.style.display !== 'none') {
            username = elements.authUsername.value.trim() || null;
        }
        // Store username for saving on success (Android auto-login)
        pendingAuthUsername = username;

        // Hash password with SHA-256, then apply challenge-response
        hashPassword(password).then(async hash => {
            // Challenge-response: SHA256(SHA256(password) + challenge)
            const challengeHash = serverChallenge ? await hashPassword(hash + serverChallenge) : hash;
            const msg = { type: 'AuthRequest', password_hash: challengeHash, request_key: false, challenge_response: !!serverChallenge };
            if (username) {
                msg.username = username;
            }
            // On reconnection, tell server which world we're viewing
            if (currentWorldIndex !== undefined) {
                msg.current_world = currentWorldIndex;
            }
            ws.send(JSON.stringify(msg));
        }).catch(err => {
            // Try fallback directly if hashPassword somehow failed
            const hash = sha256Fallback(password);
            const challengeHash = serverChallenge ? sha256Fallback(hash + serverChallenge) : hash;
            const msg = { type: 'AuthRequest', password_hash: challengeHash, request_key: false, challenge_response: !!serverChallenge };
            if (username) {
                msg.username = username;
            }
            // On reconnection, tell server which world we're viewing
            if (currentWorldIndex !== undefined) {
                msg.current_world = currentWorldIndex;
            }
            ws.send(JSON.stringify(msg));
        });
    }

    // SHA-256 hash (with fallback for insecure contexts where crypto.subtle is unavailable)
    async function hashPassword(password) {
        // Try native crypto.subtle first (only available in secure contexts)
        // Firefox throws errors on insecure contexts even when crypto.subtle exists
        if (window.crypto && window.crypto.subtle) {
            try {
                const encoder = new TextEncoder();
                const data = encoder.encode(password);
                const hashBuffer = await crypto.subtle.digest('SHA-256', data);
                const hashArray = Array.from(new Uint8Array(hashBuffer));
                return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
            } catch (err) {
                // Fall through to fallback
            }
        }
        // Fallback: pure JavaScript SHA-256 for insecure contexts (HTTP)
        return sha256Fallback(password);
    }

    // Pure JavaScript SHA-256 implementation (fallback for HTTP contexts)
    // Based on the standard FIPS 180-4 specification
    function sha256Fallback(message) {
        // Convert string to UTF-8 byte array
        const utf8 = unescape(encodeURIComponent(message));
        const bytes = [];
        for (let i = 0; i < utf8.length; i++) {
            bytes.push(utf8.charCodeAt(i));
        }

        // Constants (first 32 bits of fractional parts of cube roots of first 64 primes)
        const K = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
        ];

        // Initial hash values (first 32 bits of fractional parts of square roots of first 8 primes)
        let H = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
            0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19
        ];

        // Pre-processing: adding padding bits
        const bitLength = bytes.length * 8;
        bytes.push(0x80);
        while ((bytes.length % 64) !== 56) {
            bytes.push(0);
        }
        // Append length as 64-bit big-endian
        for (let i = 7; i >= 0; i--) {
            bytes.push((bitLength / Math.pow(2, i * 8)) & 0xff);
        }

        // Helper functions
        function rotr(x, n) { return ((x >>> n) | (x << (32 - n))) >>> 0; }
        function ch(x, y, z) { return ((x & y) ^ (~x & z)) >>> 0; }
        function maj(x, y, z) { return ((x & y) ^ (x & z) ^ (y & z)) >>> 0; }
        function sigma0(x) { return (rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)) >>> 0; }
        function sigma1(x) { return (rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)) >>> 0; }
        function gamma0(x) { return (rotr(x, 7) ^ rotr(x, 18) ^ (x >>> 3)) >>> 0; }
        function gamma1(x) { return (rotr(x, 17) ^ rotr(x, 19) ^ (x >>> 10)) >>> 0; }

        // Process each 512-bit block
        for (let i = 0; i < bytes.length; i += 64) {
            const W = [];

            // Prepare message schedule
            for (let t = 0; t < 16; t++) {
                W[t] = (bytes[i + t * 4] << 24) | (bytes[i + t * 4 + 1] << 16) |
                       (bytes[i + t * 4 + 2] << 8) | bytes[i + t * 4 + 3];
                W[t] = W[t] >>> 0;
            }
            for (let t = 16; t < 64; t++) {
                W[t] = (gamma1(W[t - 2]) + W[t - 7] + gamma0(W[t - 15]) + W[t - 16]) >>> 0;
            }

            // Initialize working variables
            let [a, b, c, d, e, f, g, h] = H;

            // Main loop
            for (let t = 0; t < 64; t++) {
                const T1 = (h + sigma1(e) + ch(e, f, g) + K[t] + W[t]) >>> 0;
                const T2 = (sigma0(a) + maj(a, b, c)) >>> 0;
                h = g;
                g = f;
                f = e;
                e = (d + T1) >>> 0;
                d = c;
                c = b;
                b = a;
                a = (T1 + T2) >>> 0;
            }

            // Update hash values
            H[0] = (H[0] + a) >>> 0;
            H[1] = (H[1] + b) >>> 0;
            H[2] = (H[2] + c) >>> 0;
            H[3] = (H[3] + d) >>> 0;
            H[4] = (H[4] + e) >>> 0;
            H[5] = (H[5] + f) >>> 0;
            H[6] = (H[6] + g) >>> 0;
            H[7] = (H[7] + h) >>> 0;
        }

        // Convert to hex string
        return H.map(h => h.toString(16).padStart(8, '0')).join('');
    }

    // Send command - all commands are sent to the server for parsing via Rust's
    // parse_command(). Server handles data commands directly and responds with
    // ExecuteLocalCommand for UI/popup commands.
    function sendCommand() {
        const cmd = elements.input.value;
        if (!authenticated) return;

        // Release all pending lines when sending a command
        if (paused) {
            releaseAll();
        }

        // Reset lines since pause counter on user input
        linesSincePause = 0;

        // Clear splash on user input (server will also clear and send WorldFlushed)
        if (worlds[currentWorldIndex] && worlds[currentWorldIndex].showing_splash) {
            worlds[currentWorldIndex].showing_splash = false;
            renderOutput();
        }

        // Intercept /update locally — should run on the client, not the server
        const cmdTrimmed = cmd.trim();
        if (cmdTrimmed === '/update' || cmdTrimmed.startsWith('/update ')) {
            elements.input.value = '';
            executeLocalCommand(cmdTrimmed);
            return;
        }

        // Intercept /window locally — open new browser/tab directly from client
        if (cmdTrimmed === '/window' || cmdTrimmed.startsWith('/window ')) {
            elements.input.value = '';
            var winArgs = cmdTrimmed.length > 8 ? cmdTrimmed.substring(8).trim() : '';

            // Check for --grep flag: /window --grep pattern [-w world] [--regexp]
            // Supports quoted patterns: --grep "*some pattern*" or --grep pattern
            var grepMatch = winArgs.match(/--grep\s+"([^"]+)"/) || winArgs.match(/--grep\s+'([^']+)'/) || winArgs.match(/--grep\s+(\S+)/);
            if (grepMatch) {
                var grepPattern = grepMatch[1];
                var grepWorldMatch = winArgs.match(/-w\s+(\S+)/);
                var grepWorld = grepWorldMatch ? grepWorldMatch[1] : null;
                var grepRegexp = winArgs.includes('--regexp');

                if (window.ipc && window.ipc.postMessage) {
                    window.ipc.postMessage('grep-window:' + JSON.stringify({
                        pattern: grepPattern,
                        world: grepWorld,
                        regex: grepRegexp
                    }));
                } else {
                    // Browser mode: open new tab with grep params in URL
                    var winProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                    var winHost = window.WS_HOST || window.location.hostname;
                    var winPort = (window.WS_PORT && window.WS_PORT !== 0)
                        ? window.WS_PORT : window.location.port;
                    var grepUrl = winProto + '://' + winHost + ':' + winPort + '/'
                        + '?grep=' + encodeURIComponent(grepPattern)
                        + (grepWorld ? '&world=' + encodeURIComponent(grepWorld) : '')
                        + (grepRegexp ? '&regexp=1' : '');
                    window.open(grepUrl, '_blank');
                }
                return;
            }

            var winWorld = winArgs;
            var winParam = winWorld ? '?world=' + encodeURIComponent(winWorld) : '';
            // Build URL using real server address (SERVER_URL set by WebView, else use WS_HOST)
            var winUrl;
            if (window.SERVER_URL) {
                winUrl = window.SERVER_URL + '/' + winParam;
            } else {
                var winProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                var winHost = window.WS_HOST || window.location.hostname;
                var winPort = (window.WS_PORT && window.WS_PORT !== 0)
                    ? window.WS_PORT : window.location.port;
                winUrl = winProto + '://' + winHost + ':' + winPort + '/' + winParam;
            }
            // In WebView mode, use IPC to spawn a new WebView window (not system browser)
            if (window.ipc && window.ipc.postMessage) {
                window.ipc.postMessage('new-window:' + (winWorld || ''));
            } else {
                window.open(winUrl, '_blank');
            }
            return;
        }

        // Intercept /reload in remote WebView mode — restart the local GUI binary
        // (master WebView passes through to server which handles exec_reload with state)
        if (window.WEBVIEW_MODE && !window.AUTO_PASSWORD &&
            (cmdTrimmed === '/reload' || cmdTrimmed.startsWith('/reload '))) {
            elements.input.value = '';
            window.ipc.postMessage('reload');
            return;
        }

        const sent = send({
            type: 'SendCommand',
            world_index: currentWorldIndex,
            command: cmd
        });

        if (!sent) {
            // Connection lost - show reconnect popup
            pendingReconnectCommand = cmd;
            pendingReconnectWorldIndex = currentWorldIndex;
            showReconnectModal();
            return;
        }

        if (cmd.length > 0) {
            commandHistory.push(cmd);
            if (commandHistory.length > 1000) {
                commandHistory.shift();
            }
        }
        historyIndex = -1;
        elements.input.value = '';
        elements.prompt.textContent = '';
    }

    // Navigate to previous command in history
    function historyPrev() {
        if (commandHistory.length > 0) {
            if (historyIndex === -1) {
                historyIndex = commandHistory.length - 1;
            } else if (historyIndex > 0) {
                historyIndex--;
            }
            elements.input.value = commandHistory[historyIndex];
        }
    }

    // Navigate to next command in history
    function historyNext() {
        if (historyIndex !== -1) {
            if (historyIndex < commandHistory.length - 1) {
                historyIndex++;
                elements.input.value = commandHistory[historyIndex];
            } else {
                historyIndex = -1;
                elements.input.value = '';
            }
        }
    }

    // Helper: check if character is a word character (A-Z, a-z, 0-9)
    function isWordChar(ch) {
        return /[A-Za-z0-9]/.test(ch);
    }

    // Transform word forward from cursor: capitalize, lowercase, or uppercase.
    // Moves cursor to end of next word, skipping trailing spaces.
    function transformWordForward(mode) {
        const input = elements.input;
        const text = input.value;
        let pos = input.selectionStart;
        let result = text.substring(0, pos);
        let i = pos;
        let atWordStart = true;
        // Skip leading non-word characters (pass through unchanged)
        while (i < text.length && !isWordChar(text[i])) {
            result += text[i];
            i++;
        }
        // Transform word characters
        while (i < text.length && isWordChar(text[i])) {
            if (mode === 'capitalize') {
                result += atWordStart ? text[i].toUpperCase() : text[i].toLowerCase();
            } else if (mode === 'uppercase') {
                result += text[i].toUpperCase();
            } else {
                result += text[i].toLowerCase();
            }
            atWordStart = false;
            i++;
        }
        // Skip trailing spaces
        while (i < text.length && text[i] === ' ') {
            result += text[i];
            i++;
        }
        const newPos = result.length;
        result += text.substring(i);
        input.value = result;
        input.selectionStart = input.selectionEnd = newPos;
    }

    // Delete forward to end of next word (Esc+D).
    // Deletes non-word chars, then word chars.
    function deleteWordForward() {
        const input = elements.input;
        const text = input.value;
        const pos = input.selectionStart;
        let i = pos;
        // Skip non-word characters
        while (i < text.length && !isWordChar(text[i])) i++;
        // Skip word characters
        while (i < text.length && isWordChar(text[i])) i++;
        input.value = text.substring(0, pos) + text.substring(i);
        input.selectionStart = input.selectionEnd = pos;
    }

    // Transpose two characters before cursor (Ctrl+T)
    function transposeChars() {
        const input = elements.input;
        const text = input.value;
        const pos = input.selectionStart;
        if (text.length < 2 || pos === 0) return;
        let a, b;
        if (pos >= text.length) {
            a = text.length - 2; b = text.length - 1;
        } else {
            a = pos - 1; b = pos;
        }
        const chars = text.split('');
        const tmp = chars[a]; chars[a] = chars[b]; chars[b] = tmp;
        input.value = chars.join('');
        input.selectionStart = input.selectionEnd = b + 1;
    }

    // Collapse multiple spaces around cursor to one (Esc+Space)
    function collapseSpaces() {
        const input = elements.input;
        const text = input.value;
        const pos = input.selectionStart;
        let start = pos;
        while (start > 0 && text[start - 1] === ' ') start--;
        let end = pos;
        while (end < text.length && text[end] === ' ') end++;
        if (end - start <= 1) return;
        input.value = text.substring(0, start) + ' ' + text.substring(end);
        input.selectionStart = input.selectionEnd = start + 1;
    }

    // Insert last word of previous history entry (Esc+. / Esc+_)
    function lastArgument() {
        if (commandHistory.length === 0) return;
        const prev = commandHistory[commandHistory.length - 1];
        const words = prev.trim().split(/\s+/);
        if (words.length === 0) return;
        const word = words[words.length - 1];
        const input = elements.input;
        const text = input.value;
        const pos = input.selectionStart;
        input.value = text.substring(0, pos) + word + text.substring(pos);
        input.selectionStart = input.selectionEnd = pos + word.length;
    }

    // Move cursor to matching bracket (Esc+-)
    function gotoMatchingBracket() {
        const input = elements.input;
        const text = input.value;
        const pos = input.selectionStart;
        if (pos >= text.length) return;
        const ch = text[pos];
        const pairs = {'(': ['(', ')', true], '[': ['[', ']', true], '{': ['{', '}', true],
                        ')': ['(', ')', false], ']': ['[', ']', false], '}': ['{', '}', false]};
        const pair = pairs[ch];
        if (!pair) return;
        const [open, close, forward] = pair;
        let depth = 0;
        if (forward) {
            for (let i = pos; i < text.length; i++) {
                if (text[i] === open) depth++;
                if (text[i] === close) depth--;
                if (depth === 0) { input.selectionStart = input.selectionEnd = i; return; }
            }
        } else {
            for (let i = pos; i >= 0; i--) {
                if (text[i] === close) depth++;
                if (text[i] === open) depth--;
                if (depth === 0) { input.selectionStart = input.selectionEnd = i; return; }
            }
        }
    }

    // Delete word backward stopping at punctuation boundaries (Esc+Backspace)
    function backwardKillWordPunctuation() {
        const input = elements.input;
        const text = input.value;
        let pos = input.selectionStart;
        if (pos === 0) return;
        // Skip whitespace
        while (pos > 0 && text[pos - 1] === ' ') pos--;
        const endPos = pos;
        if (pos > 0) {
            const last = text[pos - 1];
            if (/[a-zA-Z0-9]/.test(last)) {
                while (pos > 0 && /[a-zA-Z0-9]/.test(text[pos - 1])) pos--;
            } else {
                while (pos > 0 && !/[a-zA-Z0-9\s]/.test(text[pos - 1])) pos--;
            }
        }
        input.value = text.substring(0, pos) + text.substring(input.selectionStart);
        input.selectionStart = input.selectionEnd = pos;
    }

    // History search state
    let searchPrefix = null;
    let searchIndex = -1;

    function clearHistorySearch() {
        searchPrefix = null;
        searchIndex = -1;
    }

    // Search history backward for entries starting with current prefix (Esc+p)
    function historySearchBackward() {
        if (commandHistory.length === 0) return;
        if (searchPrefix === null) {
            searchPrefix = elements.input.value;
            searchIndex = commandHistory.length;
        }
        for (let i = searchIndex - 1; i >= 0; i--) {
            if (commandHistory[i].startsWith(searchPrefix)) {
                searchIndex = i;
                elements.input.value = commandHistory[i];
                elements.input.selectionStart = elements.input.selectionEnd = commandHistory[i].length;
                return;
            }
        }
    }

    // Search history forward for entries starting with current prefix (Esc+n)
    function historySearchForward() {
        if (searchPrefix === null) return;
        for (let i = searchIndex + 1; i < commandHistory.length; i++) {
            if (commandHistory[i].startsWith(searchPrefix)) {
                searchIndex = i;
                elements.input.value = commandHistory[i];
                elements.input.selectionStart = elements.input.selectionEnd = commandHistory[i].length;
                return;
            }
        }
        // Past end: restore original
        elements.input.value = searchPrefix;
        elements.input.selectionStart = elements.input.selectionEnd = searchPrefix.length;
        searchIndex = commandHistory.length;
    }

    // Send selective flush command
    function selectiveFlush() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({
                type: 'SelectiveFlush',
                world_index: currentWorldIndex
            }));
        }
    }

    // Execute a command locally (called from server via ExecuteLocalCommand message).
    // The server's parse_command() is the single source of truth for command parsing.
    // This function only handles the UI/popup side of commands.
    function executeLocalCommand(cmd) {
        const trimmed = cmd.trim();
        const parts = trimmed.split(/\s+/);
        const firstWord = parts[0].toLowerCase();
        const args = parts.slice(1);

        switch (firstWord) {
            case '/actions':
                openActionsListPopup(args.join(' ') || null);
                break;

            case '/web':
                openSettingsPopup('web');
                break;

            case '/setup':
                openSettingsPopup('general');
                break;

            case '/connections':
            case '/l':
                outputWorldsList();
                break;

            case '/worlds':
            case '/world':
                if (args.length === 0) {
                    openWorldSelectorPopup();
                } else if (args[0] === '-e') {
                    // /worlds -e [name] - open world editor
                    const name = args.length > 1 ? args.slice(1).join(' ') : null;
                    if (name) {
                        const idx = worlds.findIndex(w => w.name.toLowerCase() === name.toLowerCase());
                        if (idx >= 0) openWorldEditorPopup(idx);
                    } else {
                        openWorldEditorPopup(currentWorldIndex);
                    }
                } else if (args[0] === '-l') {
                    // /worlds -l <name> - server already connected, just switch local view
                    if (args.length > 1) {
                        const name = args.slice(1).join(' ');
                        const idx = worlds.findIndex(w => w.name.toLowerCase() === name.toLowerCase());
                        if (idx >= 0) switchWorldLocal(idx);
                    }
                } else {
                    // /worlds <name> - server already connected if needed, switch local view
                    const name = args.join(' ');
                    const idx = worlds.findIndex(w => w.name.toLowerCase() === name.toLowerCase());
                    if (idx >= 0) switchWorldLocal(idx);
                }
                break;

            case '/help':
                openHelpPopup();
                break;

            case '/menu':
                openMenuPopup();
                break;

            case '/font':
                openSettingsPopup('font');
                break;

            case '/note':
                // Open split-screen editor locally
                // (handled by specific client-side logic if implemented)
                break;

            case '/quit':
                // Close the window — use IPC for WebView GUI, window.close() for browser
                if (window.ipc && window.ipc.postMessage) {
                    window.ipc.postMessage('quit');
                } else {
                    window.close();
                }
                break;

            case '/reload':
                // Hot reload — local only, never restart the remote server
                if (window.ipc && window.ipc.postMessage) {
                    window.ipc.postMessage('reload');
                } else {
                    window.location.reload();
                }
                break;

            case '/update':
                // Update the local client binary
                if (window.ipc && window.ipc.postMessage) {
                    // WebView GUI: delegate to native side via IPC
                    const forceFlag = args.length > 0 && (args[0] === '-f' || args[0] === '--force');
                    window.ipc.postMessage(forceFlag ? 'update-force' : 'update');
                    appendClientLine('Checking for updates...');
                } else {
                    // Browser: can't update
                    appendClientLine('Update is only available in the desktop app. Download the latest version from https://github.com/c-hudson/clay/releases');
                }
                break;

            default:
                // For commands not handled locally, send to server
                send({
                    type: 'SendCommand',
                    world_index: currentWorldIndex,
                    command: cmd
                });
                break;
        }
    }

    // Switch world locally (does not affect console)
    function switchWorldLocal(index) {
        if (lockedWorld) return; // Don't switch worlds in locked windows
        if (index >= 0 && index < worlds.length && index !== currentWorldIndex) {
            mcmpStopAll();
            // Clear new line indicators on the world we're LEAVING (matches console behavior)
            const oldWorld = worlds[currentWorldIndex];
            if (oldWorld && oldWorld.output_lines) {
                oldWorld.output_lines.forEach(l => { l.marked_new = false; });
            }
            currentWorldIndex = index;
            // Clear splash on world switch — if the world has output, show it
            const newWorld = worlds[index];
            if (newWorld && newWorld.showing_splash && newWorld.output_lines && newWorld.output_lines.length > 0) {
                newWorld.showing_splash = false;
            }
            // Reset more-mode state for new world
            paused = false;
            pendingLines = [];
            linesSincePause = 0;
            renderOutput();
            updateStatusBar();
            // Update prompt to show new world's prompt
            const world = worlds[currentWorldIndex];
            if (world && world.prompt) {
                elements.prompt.innerHTML = sanitizeHtml(parseAnsi(world.prompt));
            } else {
                elements.prompt.textContent = '';
            }
            // Notify server that this world has been seen (syncs unseen count)
            send({ type: 'MarkWorldSeen', world_index: index });
            // Request current state for this world (more indicator, prompt, etc)
            send({ type: 'RequestWorldState', world_index: index });
            // Update view state for synchronized more-mode
            sendViewStateIfChanged();
        }
    }

    // Render output - render all lines as text with line breaks
    // Filter popup functions
    function openFilterPopup() {
        filterPopupOpen = true;
        filterText = '';
        elements.filterPopup.style.display = 'block';
        elements.filterInput.value = '';
        elements.filterInput.focus();
    }

    function closeFilterPopup() {
        filterPopupOpen = false;
        filterText = '';
        elements.filterPopup.style.display = 'none';
        elements.input.focus();
        renderOutput();
    }

    function updateFilter() {
        filterText = elements.filterInput.value;
        renderOutput();
    }

    // Help popup functions (/help)
    // Help content as structured sections: [heading, [left, right], ...]
    // Empty right = continuation line; null right = section heading
    const helpSections = [
        { heading: 'Commands', note: '(/help &lt;command&gt; for details)', rows: [
            { heading: 'Connection' },
            { l: '/worlds', r: 'Open world selector' },
            { l: '/worlds &lt;name&gt;', r: 'Connect to or create world' },
            { l: '/worlds -e [name]', r: 'Edit world settings' },
            { l: '/worlds -l &lt;name&gt;', r: 'Connect without auto-login' },
            { l: '/disconnect (or /dc)', r: 'Disconnect from server' },
            { l: '/connections (or /l)', r: 'List connected worlds' },
            { heading: 'Communication' },
            { l: '/send [-W] [-w&lt;world&gt;] [-n] &lt;text&gt;', r: 'Send text to world(s)' },
            { l: '', r: '-W=all worlds, -n=no newline' },
            { l: '/notify &lt;message&gt;', r: 'Send notification to mobile' },
            { heading: 'Lookup &amp; Translation' },
            { l: '/dict &lt;prefix&gt; &lt;word&gt;', r: 'Look up word definition' },
            { l: '/urban &lt;prefix&gt; &lt;word&gt;', r: 'Look up Urban Dictionary' },
            { l: '/translate &lt;lang&gt; &lt;prefix&gt; &lt;text&gt;', r: 'Translate text (or /tr)' },
            { heading: 'Actions &amp; Triggers' },
            { l: '/actions [world]', r: 'Open actions editor' },
            { l: '/gag [pattern]', r: 'List gags, or gag lines matching pattern' },
            { l: '/&lt;action_name&gt; [args]', r: 'Execute named action' },
            { heading: 'Settings' },
            { l: '/setup', r: 'Open global settings' },
            { l: '/web', r: 'Open web/WebSocket settings' },
            { l: '/tag', r: 'Toggle MUD tag display (F2)' },
            { heading: 'Display' },
            { l: '/menu', r: 'Open menu popup' },
            { l: '/flush', r: 'Clear output buffer' },
            { l: '/dump', r: 'Dump scrollback to file' },
            { l: '/note [file]', r: 'Open split-screen editor' },
            { heading: 'System' },
            { l: '/help [topic]', r: 'Show help (topic = command)' },
            { l: '/version', r: 'Show version info' },
            { l: '/reload', r: 'Hot reload binary' },
            { l: '/testmusic', r: 'Test ANSI music playback' },
            { l: '/quit', r: 'Exit client' },
            { heading: 'Security' },
            { l: '/ban', r: 'Show banned hosts' },
            { l: '/unban &lt;host&gt;', r: 'Remove host from ban list' },
        ]},
        { heading: 'TF Commands', rows: [
            { l: 'For TinyFugue commands (triggers, macros,', r: '' },
            { l: 'variables, control flow): /help tf or /tfhelp', r: '' },
        ]},
        { heading: 'World Switching', rows: [
            { l: 'Up/Down', r: 'Switch between active worlds' },
            { l: 'Ctrl+Up/Down', r: 'Switch between all worlds' },
            { l: 'Alt+W', r: 'Switch to world with activity' },
        ]},
        { heading: 'Input', rows: [
            { l: 'Left/Right, Ctrl+B/F', r: 'Move cursor' },
            { l: 'Ctrl+Up/Down', r: 'Move cursor up/down lines' },
            { l: 'Alt+Up/Down', r: 'Resize input area' },
            { l: 'Ctrl+U', r: 'Clear input' },
            { l: 'Ctrl+W', r: 'Delete word before cursor' },
            { l: 'Ctrl+K', r: 'Delete to end of line' },
            { l: 'Ctrl+D', r: 'Delete character under cursor' },
            { l: 'Ctrl+A/Home', r: 'Jump to start of line' },
            { l: 'Ctrl+E/End', r: 'Jump to end of line' },
            { l: 'Esc+D', r: 'Delete word forward' },
            { l: 'Esc+C / Esc+L / Esc+U', r: 'Capitalize / Lower / Upper' },
            { l: 'Ctrl+P/N', r: 'Command history' },
            { l: 'Ctrl+Q', r: 'Spell suggestions' },
            { l: 'Tab', r: 'Command completion' },
        ]},
        { heading: 'Output', rows: [
            { l: 'PageUp/PageDown', r: 'Scroll output' },
            { l: 'Tab', r: 'Release one screenful (paused)' },
            { l: 'Alt+J', r: 'Jump to end, release all' },
            { l: 'Esc+H', r: 'Half-page scroll/release' },
        ]},
        { heading: 'Display', rows: [
            { l: 'F1', r: 'Show this help' },
            { l: 'F2', r: 'Toggle MUD tag display' },
            { l: 'F4', r: 'Filter output' },
            { l: 'F8', r: 'Highlight action matches' },
            { l: 'F9', r: 'Toggle GMCP media audio' },
        ]},
    ];

    function getBaseUrl() {
        const proto = window.location.protocol; // 'http:' or 'https:'
        const host = window.location.hostname;
        const port = window.location.port;
        // Use origin if port matches default, otherwise include port
        if (port && port !== '80' && port !== '443') {
            return proto + '//' + host + ':' + port;
        }
        return proto + '//' + host;
    }

    function openHelpPopup() {
        helpPopupOpen = true;
        const baseUrl = getBaseUrl();
        let html = '<table class="help-table">';
        for (const section of helpSections) {
            html += '<tr><td colspan="2" class="help-section-heading">' + section.heading;
            if (section.note) html += ' <span class="help-note">' + section.note + '</span>';
            html += '</td></tr>';
            for (const row of section.rows) {
                if (row.heading) {
                    html += '<tr><td colspan="2" class="help-sub-heading">' + row.heading + '</td></tr>';
                } else {
                    html += '<tr><td class="help-left">' + row.l + '</td><td class="help-right">' + row.r + '</td></tr>';
                }
            }
            html += '<tr><td colspan="2">&nbsp;</td></tr>';
        }
        // Editor links
        html += '<tr><td colspan="2" class="help-section-heading">Editors</td></tr>';
        html += '<tr><td class="help-left"><a href="' + baseUrl + '/theme-editor" target="_blank">Theme Editor</a></td>';
        html += '<td class="help-right">Customize GUI/web colors</td></tr>';
        html += '<tr><td class="help-left"><a href="' + baseUrl + '/keybind-editor" target="_blank">Keybind Editor</a></td>';
        html += '<td class="help-right">Configure keyboard bindings</td></tr>';
        html += '</table>';
        elements.helpContent.innerHTML = html;
        elements.helpModal.classList.add('visible');
    }

    function closeHelpPopup() {
        helpPopupOpen = false;
        elements.helpModal.classList.remove('visible');
        elements.input.focus();
    }

    // Popup-specific help texts
    const popupHelpTexts = {
        setup: [
            'Setup - Global Settings', '',
            'World Switching: Controls Up/Down world switch order.',
            '  "Unseen First" prioritizes worlds with new activity.',
            '  "Alphabetical" cycles worlds in name order.', '',
            'Theme: Dark or Light theme for web/GUI clients.', '',
            'Color Offset: Shifts the base ANSI color palette.', '',
            'Input Height: Number of input lines visible (1-10).', '',
            'More Mode: Pauses output when a full screen of text',
            '  arrives. Keeps you from missing important text.', '',
            'TLS Proxy: Keeps a proxy alive during hot reload', '  so TLS connections survive.', '',
            'New Indicator: Show a marker on new lines arriving', '  while scrolled up in the output buffer.', '',
            'Debug: Enables debug logging to clay.debug.log.', '',
            'ANSI Music: Play ANSI music sequences from MUDs.', '',
            'ZWJ Sequence: For terminals that support combined',
            '  emoji (ZWJ). If unsupported, shows two separate',
            '  emoji instead of one combined one.'
        ],
        web: [
            'Web Settings - Remote Access', '',
            'These settings let you access Clay from a web',
            'browser or mobile device on your network.', '',
            'Protocol: Choose Secure (HTTPS/WSS) or Non-Secure',
            '  (HTTP/WS). Secure requires TLS certificate files.', '',
            'HTTP Enabled: Starts a web server so you can open',
            '  Clay in a browser at http://yourhost:port.', '',
            'HTTP Port: The port number for the web server.', '',
            'Allow List: Comma-separated IP addresses or',
            '  subnets allowed to connect. Empty = allow all.', '',
            'TLS Cert/Key File: Paths to your TLS/SSL certificate',
            '  and private key files for secure connections.'
        ],
        worldEditor: [
            'World Settings - Configure a Connection', '',
            'Name: A unique name for this connection.', '',
            'Hostname: The server address (e.g. mud.example.com).', '',
            'Port: The server port number (e.g. 4000, 23).', '',
            'User: Your character/login name. Used for auto-login.', '',
            'Password: Your password. Used for auto-login.', '',
            'Use SSL: Enable TLS/SSL encryption for the connection.', '',
            'Auto Login: How to send credentials on connect.',
            '  Connect: Send "connect user password".',
            '  Prompt: Wait for prompts, send user then password.',
            '  None: Don\'t auto-login.', '',
            'Keep Alive: Prevents idle disconnects.',
            '  NOP: Sends a telnet NOP (invisible to server).',
            '  Custom: Sends a custom command you specify.', '',
            'Encoding: UTF-8 (modern), Latin-1 (older MUDs), FANSI.', '',
            'GMCP: Space-separated GMCP packages to request.'
        ],
        worldSelector: [
            'World Selector - Browse and Connect', '',
            'Shows all configured worlds. Connected worlds are',
            'highlighted with a green dot.', '',
            'Filter: Type to search worlds by name or hostname.', '',
            'Connected toggle: Show only connected worlds.', '',
            'Add: Create a new world.',
            'Edit: Edit the selected world\'s settings.',
            'Connect: Connect to the selected world.',
            'Close: Close without action.'
        ],
        actionsList: [
            'Actions - Triggers and Automation', '',
            'Actions automatically respond to MUD output. When',
            'text from the MUD matches an action\'s pattern, the',
            'action\'s command is executed.', '',
            'Click an action to edit it. Use the toggle to',
            'enable or disable actions.', '',
            'Add: Create a new action.',
            'Edit: Edit the selected action.',
            'Delete: Remove the selected action.', '',
            'Use the filter to search by name, world, or pattern.'
        ],
        actionEditor: [
            'Action Editor - Configure a Trigger', '',
            'Name: A unique name for this action.', '',
            'World: Which world this applies to (blank = all).', '',
            'Match Type:',
            '  Regexp - Regular expression (e.g. ^You are (\\w+))',
            '  Wildcard - Simple wildcards (* matches anything)', '',
            'Pattern: Text to match against MUD output.',
            '  Leave empty for manual-only actions.', '',
            'Command: What to execute when pattern matches.',
            '  Multiple commands separated by semicolons (;).',
            '  Use $1-$9 for captured groups from the pattern.',
            '  /gag hides the matched line.',
            '  /notify sends a push notification.', '',
            'Enabled: Whether this action is active.', '',
            'Startup: Run command when Clay starts/hot-reloads.'
        ],
        connections: [
            'Connected Worlds - Active Connections', '',
            'Shows all currently connected worlds.', '',
            'Columns:',
            '  World  - Name of the connected world',
            '  Unseen - Lines received since you last viewed',
            '  Last   - Time since last send/receive',
            '  KA     - Time until next keep-alive packet',
            '  Buffer - Number of lines in output buffer', '',
            'Click a world to switch to it.'
        ],
        menu: [
            'Menu - Quick Access', '',
            'Select an item to open it.', '',
            '  Help           - Keyboard shortcuts and commands',
            '  Settings       - Global application settings',
            '  Web Settings   - WebSocket/HTTP server config',
            '  Actions        - Trigger and automation editor',
            '  World Selector - Browse and connect to worlds',
            '  Connected Worlds - View active connections'
        ],
        'clay-server': [
            'Clay Server - Connection Settings', '',
            'Host: The local IP or hostname of your Clay server',
            '  (e.g. 192.168.1.100).', '',
            'Remote Host: An optional WAN hostname or external IP',
            '  for connecting when away from your local network',
            '  (e.g. myhost.example.com).', '',
            '  When Remote Host is set, Clay first attempts the',
            '  local Host. If unreachable (2s timeout), it falls',
            '  back to Remote Host automatically.', '',
            '  Leave Remote Host empty to always use Host.', '',
            'Port: The Clay web server port (default 9000).', '',
            'Username / Password: Your Clay login credentials.', '',
            'Auth Key: Used for passwordless login to Clay.',
            '  Paste a key here manually, or tap Download when',
            '  connected to fetch the key from the server and',
            '  store it in the app for future logins.'
        ]
    };

    function openPopupHelp(key) {
        const lines = popupHelpTexts[key];
        if (!lines) return;
        let html = '<div style="white-space:pre-wrap;font-family:var(--font-mono);font-size:13px;line-height:1.5;padding:4px 8px;text-align:left">';
        for (const line of lines) {
            html += escapeHtml(line) + '\n';
        }
        html += '</div>';
        elements.popupHelpContent.innerHTML = html;
        elements.popupHelpModal.classList.add('visible');
    }

    function closePopupHelp() {
        elements.popupHelpModal.classList.remove('visible');
    }

    // Menu popup functions (/menu)
    function openMenuPopup() {
        menuPopupOpen = true;
        menuSelectedIndex = 0;
        elements.menuModal.classList.add('visible');
        updateMenuSelection();
    }

    function closeMenuPopup() {
        menuPopupOpen = false;
        elements.menuModal.classList.remove('visible');
        elements.input.focus();
    }

    function updateMenuSelection() {
        const items = elements.menuList.querySelectorAll('.menu-item');
        items.forEach((item, i) => {
            if (i === menuSelectedIndex) {
                item.classList.add('selected');
            } else {
                item.classList.remove('selected');
            }
        });
    }

    function selectMenuItem() {
        const cmd = menuItems[menuSelectedIndex].command;
        closeMenuPopup();
        elements.input.value = cmd;
        sendCommand();
    }

    // Strip ANSI codes for filter matching
    function stripAnsiForFilter(text) {
        return text.replace(/\x1b\[[0-9;?]*[@-~]/g, '');
    }

    // Convert wildcard filter pattern to regex for F4 filter popup
    // Always uses "contains" semantics - patterns match anywhere in the line
    // * matches any sequence, ? matches any single character
    // Supports \* and \? to match literal asterisk and question mark
    function filterWildcardToRegex(pattern) {
        let regex = '';
        // No anchoring - always "contains" semantics for filter

        let i = 0;
        while (i < pattern.length) {
            const c = pattern[i];
            if (c === '\\' && i + 1 < pattern.length) {
                const next = pattern[i + 1];
                if (next === '*' || next === '?' || next === '\\') {
                    // Escaped wildcard or backslash - treat as literal
                    regex += '\\' + next;
                    i += 2;
                    continue;
                }
            }
            if (c === '*') {
                regex += '.*';
            } else if (c === '?') {
                regex += '.';
            } else if ('.+^$|\\()[]{}'.includes(c)) {
                regex += '\\' + c;
            } else {
                regex += c;
            }
            i++;
        }

        try {
            return new RegExp(regex, 'i');
        } catch (e) {
            return null;
        }
    }

    // Check if text matches filter pattern (supports wildcards * and ?)
    function matchesFilter(text, pattern) {
        const hasWildcards = pattern.includes('*') || pattern.includes('?');
        if (hasWildcards) {
            const regex = filterWildcardToRegex(pattern);
            return regex ? regex.test(text) : false;
        } else {
            // Simple case-insensitive substring match
            return text.toLowerCase().includes(pattern.toLowerCase());
        }
    }

    // Check if a line matches any action pattern (for F8 highlighting)
    function lineMatchesAction(line, worldName) {
        const plainLine = stripAnsiForFilter(line).toLowerCase();
        for (const action of actions) {
            // Skip disabled actions
            if (action.enabled === false) continue;
            // Skip actions without patterns
            if (!action.pattern || action.pattern.trim() === '') continue;
            // Check world match (empty = all worlds)
            if (action.world && action.world.trim() !== '' &&
                action.world.toLowerCase() !== worldName.toLowerCase()) continue;
            // Convert pattern based on match type
            try {
                let pattern = action.pattern;
                if (action.match_type === 'wildcard') {
                    pattern = filterWildcardToRegex(action.pattern);
                }
                const regex = new RegExp(pattern, 'i');
                if (regex.test(plainLine)) return true;
            } catch (e) {
                // Invalid regex, skip
            }
        }
        return false;
    }

    // Get raw ANSI text for given line indices (used by WebView debug selection)
    window.getDebugSelectionText = function(lineIndices) {
        var world = worlds[currentWorldIndex];
        if (!world) return '';
        var lines = world.output_lines || [];
        var parts = [];
        for (var i = 0; i < lineIndices.length; i++) {
            var idx = lineIndices[i];
            if (idx >= 0 && idx < lines.length) {
                var lineObj = lines[idx];
                var raw = typeof lineObj === 'string' ? lineObj : lineObj.text;
                parts.push(String(raw).replace(/\x1b/g, '<esc>'));
            }
        }
        return parts.join('\n');
    };

    // Render splash screen in output area
    function renderSplashScreen() {
        if (!splashLines || splashLines.length === 0) return;

        // Just render splash lines as regular output
        const htmlParts = [];
        for (const line of splashLines) {
            const lineHtml = parseAnsi(line);
            htmlParts.push(lineHtml);
        }
        elements.output.innerHTML = htmlParts.join('<br>');
    }

    function renderOutput() {
        const world = worlds[currentWorldIndex];

        // If no world selected (multiuser mode before connecting), show splash
        if (!world) {
            if (splashLines && splashLines.length > 0) {
                renderSplashScreen();
            }
            return;
        }

        // WebView mode: show image splash instead of text splash
        if (window.WEBVIEW_MODE && world.showing_splash) {
            // On Windows WebView2, custom protocol "clay://" is served as "http://clay.localhost/"
            const imgBase = window.location.origin || 'clay://localhost';
            elements.output.innerHTML = '<div style="display:flex;flex-direction:column;align-items:center;justify-content:center;height:100%;gap:5px;">' +
                '<img src="' + imgBase + '/clay2.png" alt="Clay" style="width:200px;height:200px;">' +
                '<div style="color:#ff87ff;font-style:italic;">A 90dies mud client written today</div>' +
                '<div style="color:#888;">/help for how to use clay</div>' +
                '</div>';
            return;
        }

        const lines = world.output_lines || [];

        // Limit initial render to last 500 lines to avoid overwhelming WebKitGTK
        // Full scrollback is available via PageUp which triggers a re-render
        const maxRenderLines = 500;
        const startIdx = Math.max(0, lines.length - maxRenderLines);

        // Build lines as HTML with explicit <br> line breaks
        const htmlParts = [];
        for (let i = startIdx; i < lines.length; i++) {
            const lineObj = lines[i];
            if (lineObj === undefined || lineObj === null) continue;

            // Handle both old string format and new object format
            const rawLine = typeof lineObj === 'string' ? lineObj : lineObj.text;
            const lineTs = typeof lineObj === 'object' ? lineObj.ts : null;
            const lineGagged = typeof lineObj === 'object' ? lineObj.gagged : false;
            const lineHighlightColor = typeof lineObj === 'object' ? lineObj.highlight_color : null;
            const lineMarkedNew = typeof lineObj === 'object' ? lineObj.marked_new : false;

            // Skip gagged lines unless showTags is enabled (F2)
            if (lineGagged && !showTags) {
                continue;
            }

            // Strip newlines/carriage returns
            const cleanLine = String(rawLine).replace(/[\r\n]+/g, '');

            // Filter: skip lines that don't match (case-insensitive)
            // Filter: skip lines that don't match (supports wildcards * and ?)
            if (filterPopupOpen && filterText.length > 0) {
                const plainLine = stripAnsiForFilter(cleanLine);
                if (!matchesFilter(plainLine, filterText)) {
                    continue;
                }
            }

            // Grep mode: skip lines that don't match the grep pattern
            // Match against displayed text (strip ANSI codes AND MUD tags)
            if (grepRegex) {
                const plainLine = stripMudTag(stripAnsiForFilter(cleanLine));
                if (!grepRegex.test(plainLine)) {
                    continue;
                }
            }

            // Format timestamp prefix if showTags is enabled
            const tsPrefix = showTags && lineTs ? `<span class="timestamp">${formatTimestamp(lineTs)}</span>` : '';

            const strippedText = showTags ? cleanLine : stripMudTag(cleanLine);
            const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
            // Skip Discord emoji conversion when showTags is enabled so users can see original text
            const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
            const newLinePrefix = (newLineIndicator && lineMarkedNew) ? '<span style="color:#00ff00;">▶</span> ' : '';
            let html = tsPrefix + newLinePrefix + (showTags ? processed : convertDiscordEmojis(processed));

            // Apply /highlight color from action command (takes priority)
            if (lineHighlightColor !== null && lineHighlightColor !== undefined) {
                const bgColor = colorNameToCss(lineHighlightColor);
                html = `<span style="background-color: ${bgColor}; display: block;">${html}</span>`;
            }
            // Apply F8 action highlighting if enabled (and no explicit highlight color)
            else if (highlightActions && lineMatchesAction(cleanLine, world.name || '')) {
                html = `<span class="action-highlight">${html}</span>`;
            }

            htmlParts.push(`<span data-line-idx="${i}">${html}</span>`);
        }

        // Join with <br> tags for explicit line breaks
        elements.output.innerHTML = htmlParts.join('<br>');
        scrollToBottom();

        // Clear unseen for current world
        world.unseen_lines = 0;
    }

    // Create cached HTML for a line
    function cacheLineHtml(worldIndex, lineIndex, text) {
        if (!worldOutputCache[worldIndex]) {
            worldOutputCache[worldIndex] = [];
        }
        const strippedText = showTags ? text : stripMudTag(text);
        const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
        // Skip Discord emoji conversion when showTags is enabled so users can see original text
        const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
        const html = showTags ? processed : convertDiscordEmojis(processed);
        worldOutputCache[worldIndex][lineIndex] = { html, showTags };
        return html;
    }

    // Append a client-generated message to output
    // style: 'info' (✨ prefix) or 'system' (yellow %% prefix)
    function appendClientLine(text, worldIndex = currentWorldIndex, style = 'info') {
        const prefixes = {
            info: '✨ ',
            system: '\x1b[33m%% '
        };
        const suffixes = {
            info: '',
            system: '\x1b[0m'
        };
        const prefix = prefixes[style] || prefixes.info;
        const suffix = suffixes[style] || '';
        const clientText = prefix + text + suffix;
        const ts = Math.floor(Date.now() / 1000);
        if (worldIndex >= 0 && worldIndex < worlds.length) {
            const lineIndex = worlds[worldIndex].output_lines.length;
            worlds[worldIndex].output_lines.push({ text: clientText, ts: ts, from_server: false });
            if (worldIndex === currentWorldIndex) {
                appendNewLine(clientText, ts, worldIndex, lineIndex);
            }
        }
    }

    // Append a new line to current world's output (already visible)
    function appendNewLine(text, ts, worldIndex, lineIndex, markedNew) {
        // Strip newlines/carriage returns
        const cleanText = String(text).replace(/[\r\n]+/g, '');

        // Format timestamp prefix if showTags is enabled
        const tsPrefix = showTags && ts ? `<span class="timestamp">${formatTimestamp(ts)}</span>` : '';

        const strippedText = showTags ? cleanText : stripMudTag(cleanText);
        const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
        // Skip Discord emoji conversion when showTags is enabled so users can see original text
        const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
        const newLinePrefix = (newLineIndicator && markedNew) ? '<span style="color:#00ff00;">▶</span> ' : '';
        const html = tsPrefix + newLinePrefix + (showTags ? processed : convertDiscordEmojis(processed));

        // Append to output with a <br> prefix (if not first line)
        const prefix = elements.output.childNodes.length > 0 ? '<br>' : '';
        elements.output.insertAdjacentHTML('beforeend', prefix + `<span data-line-idx="${lineIndex}">${html}</span>`);

        scheduleScrollToBottom();
    }

    // Parse ANSI escape codes (supports 16, 256, and true color)
    function parseAnsi(text) {
        // Handle various escape character representations
        // Some systems send \x1b or \u001b as literal text (double-encoded)
        // Real ESC characters (0x1B) are already correct from JSON parsing
        // Note: \e normalization removed - it falsely converts literal \e in MUD
        // output (e.g., MUSH code, regex patterns) into ESC characters
        text = text.replace(/\\x1b/gi, '\x1b');
        text = text.replace(/\\u001b/gi, '\x1b');

        // First, strip ALL ANSI CSI sequences (not just SGR)
        // This handles cursor control, screen clearing, etc.
        // CSI format: ESC [ <params> <final byte>
        // Final byte is in range 0x40-0x7E (@ through ~)
        text = text.replace(/\x1b\[[0-9;?]*[A-Za-z@`~]/g, function(match) {
            // Only keep SGR sequences (ending in 'm') for color processing
            if (match.endsWith('m')) {
                return match; // Keep for color parsing below
            }
            return ''; // Strip other CSI sequences
        });

        // Read ANSI 16-color palette from CSS theme variables (set by server)
        function getThemeAnsiPalette() {
            const fallback = [
                [0, 0, 0], [170, 0, 0], [68, 170, 68], [170, 85, 0],
                [0, 57, 170], [170, 34, 170], [26, 146, 170], [232, 228, 236],
                [119, 119, 119], [255, 135, 135], [76, 230, 76], [222, 216, 44],
                [41, 95, 204], [204, 88, 204], [76, 204, 230], [255, 255, 255]
            ];
            const style = getComputedStyle(document.documentElement);
            const palette = [];
            for (let i = 0; i < 16; i++) {
                const val = style.getPropertyValue('--theme-ansi-' + i).trim();
                if (val && val.startsWith('#') && val.length === 7) {
                    palette.push([parseInt(val.slice(1,3), 16), parseInt(val.slice(3,5), 16), parseInt(val.slice(5,7), 16)]);
                } else {
                    palette.push(fallback[i]);
                }
            }
            return palette;
        }
        let themeAnsiPalette = null;

        // 256-color palette (first 16 are standard, 16-231 are RGB cube, 232-255 are grayscale)
        function color256ToRgb(n) {
            if (n < 16) {
                // Standard 16 colors from theme
                if (!themeAnsiPalette) themeAnsiPalette = getThemeAnsiPalette();
                return themeAnsiPalette[n];
            } else if (n < 232) {
                // 216 color cube (6x6x6) - xterm uses specific values, not linear
                // The 6 levels are: 0, 95, 135, 175, 215, 255
                const cubeValues = [0, 95, 135, 175, 215, 255];
                n -= 16;
                const r = cubeValues[Math.floor(n / 36)];
                const g = cubeValues[Math.floor((n % 36) / 6)];
                const b = cubeValues[n % 6];
                return [r, g, b];
            } else {
                // Grayscale (24 shades) - starts at 8, increments by 10
                const gray = (n - 232) * 10 + 8;
                return [gray, gray, gray];
            }
        }

        // Color name to RGB mapping - reads from theme palette
        function getColorNameToRgb() {
            if (!themeAnsiPalette) themeAnsiPalette = getThemeAnsiPalette();
            const p = themeAnsiPalette;
            return {
                'black': p[0], 'red': p[1], 'green': p[2], 'yellow': p[3],
                'blue': p[4], 'magenta': p[5], 'cyan': p[6], 'white': p[7],
                'bright-black': p[8], 'bright-red': p[9], 'bright-green': p[10],
                'bright-yellow': p[11], 'bright-blue': p[12], 'bright-magenta': p[13],
                'bright-cyan': p[14], 'bright-white': p[15]
            };
        }
        let colorNameToRgb = null;

        // Get RGB from class name or style
        function getFgRgb(classes, style) {
            if (!colorNameToRgb) colorNameToRgb = getColorNameToRgb();
            // Check inline style first
            const styleMatch = style.match(/color:\s*rgb\((\d+),(\d+),(\d+)\)/);
            if (styleMatch) return [parseInt(styleMatch[1]), parseInt(styleMatch[2]), parseInt(styleMatch[3])];
            // Check class names
            for (const cls of classes) {
                if (cls.startsWith('ansi-') && !cls.startsWith('ansi-bg-') && !['ansi-bold', 'ansi-italic', 'ansi-underline'].includes(cls)) {
                    const colorName = cls.replace('ansi-', '');
                    if (colorNameToRgb[colorName]) return colorNameToRgb[colorName];
                }
            }
            return [232, 228, 236]; // Default text color (matches theme fg)
        }

        function getBgRgb(classes, style) {
            if (!colorNameToRgb) colorNameToRgb = getColorNameToRgb();
            // Check inline style first
            const styleMatch = style.match(/background-color:\s*rgb\((\d+),(\d+),(\d+)\)/);
            if (styleMatch) return [parseInt(styleMatch[1]), parseInt(styleMatch[2]), parseInt(styleMatch[3])];
            // Check class names
            for (const cls of classes) {
                if (cls.startsWith('ansi-bg-')) {
                    const colorName = cls.replace('ansi-bg-', '');
                    if (colorNameToRgb[colorName]) return colorNameToRgb[colorName];
                }
            }
            return null; // No background
        }

        // Adjust foreground color for contrast when it's too similar to background
        function adjustFgForContrast(fgRgb, bgRgb, offsetPercent) {
            if (offsetPercent === 0) return fgRgb;

            // Use theme background if no explicit background
            const effectiveBg = bgRgb || [13, 17, 23]; // Dark theme background

            // Calculate color distance (simple RGB distance)
            const dr = Math.abs(fgRgb[0] - effectiveBg[0]);
            const dg = Math.abs(fgRgb[1] - effectiveBg[1]);
            const db = Math.abs(fgRgb[2] - effectiveBg[2]);
            const distance = dr + dg + db;

            // Threshold for "too similar" - scale by color_offset_percent
            // At 100%, colors within distance 150 are adjusted
            const threshold = Math.floor((150 * offsetPercent) / 100);

            if (distance >= threshold) return fgRgb; // Colors are different enough

            // Calculate background brightness to determine if bg is light or dark
            const bgBrightness = Math.floor((effectiveBg[0] + effectiveBg[1] + effectiveBg[2]) / 3);
            const isBgDark = bgBrightness < 128;

            // Adjustment amount based on color_offset_percent
            const adjustment = Math.min(offsetPercent * 2, 200); // Max 200 adjustment

            // If background is dark, lighten foreground; if light, darken foreground
            if (isBgDark) {
                return [
                    Math.min(fgRgb[0] + adjustment, 255),
                    Math.min(fgRgb[1] + adjustment, 255),
                    Math.min(fgRgb[2] + adjustment, 255)
                ];
            } else {
                return [
                    Math.max(fgRgb[0] - adjustment, 0),
                    Math.max(fgRgb[1] - adjustment, 0),
                    Math.max(fgRgb[2] - adjustment, 0)
                ];
            }
        }

        // Blend two RGB colors
        function blendColors(fg, bg, fgWeight) {
            return [
                Math.round(fg[0] * fgWeight + bg[0] * (1 - fgWeight)),
                Math.round(fg[1] * fgWeight + bg[1] * (1 - fgWeight)),
                Math.round(fg[2] * fgWeight + bg[2] * (1 - fgWeight))
            ];
        }

        // Process shade characters - replace with solid blocks using blended colors
        function processShadeChars(text, classes, fgStyle, bgStyle) {
            const hasBg = classes.some(c => c.startsWith('ansi-bg-')) || bgStyle;
            if (!hasBg) return { wasProcessed: false }; // No background, keep as-is

            const shadeChars = /[░▒▓]/;
            if (!shadeChars.test(text)) return { wasProcessed: false }; // No shade chars

            const fgRgb = getFgRgb(classes, fgStyle);
            const bgRgb = getBgRgb(classes, bgStyle);
            if (!bgRgb) return { wasProcessed: false };

            // Pre-calculate blended colors for each shade type
            const lightBlend = blendColors(fgRgb, bgRgb, 0.25);
            const mediumBlend = blendColors(fgRgb, bgRgb, 0.5);
            const darkBlend = blendColors(fgRgb, bgRgb, 0.75);

            // Group consecutive characters by their color
            let segments = [];
            let currentSegment = { chars: '', color: null };

            for (const char of text) {
                let charColor = null;
                let outputChar = char;

                if (char === '░') {
                    charColor = `rgb(${lightBlend[0]},${lightBlend[1]},${lightBlend[2]})`;
                    outputChar = '█';
                } else if (char === '▒') {
                    charColor = `rgb(${mediumBlend[0]},${mediumBlend[1]},${mediumBlend[2]})`;
                    outputChar = '█';
                } else if (char === '▓') {
                    charColor = `rgb(${darkBlend[0]},${darkBlend[1]},${darkBlend[2]})`;
                    outputChar = '█';
                }

                // Check if we need to start a new segment
                if (charColor !== currentSegment.color) {
                    if (currentSegment.chars) {
                        segments.push({ ...currentSegment });
                    }
                    currentSegment = { chars: outputChar, color: charColor };
                } else {
                    currentSegment.chars += outputChar;
                }
            }
            if (currentSegment.chars) {
                segments.push(currentSegment);
            }

            // Build HTML from segments
            let html = '';
            const baseClasses = classes.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || ['ansi-bold', 'ansi-italic', 'ansi-underline', 'ansi-blink'].includes(c));

            for (const seg of segments) {
                const escapedChars = escapeHtml(seg.chars);
                if (seg.color) {
                    // Shade character - use blended color, keep background
                    html += `<span style="color:${seg.color};${bgStyle}">${escapedChars}</span>`;
                } else {
                    // Regular character - use original styling
                    const cls = classes.length > 0 ? ` class="${classes.join(' ')}"` : '';
                    const sty = (fgStyle || bgStyle) ? ` style="${fgStyle}${bgStyle}"` : '';
                    html += `<span${cls}${sty}>${escapedChars}</span>`;
                }
            }

            return { processedHtml: html, wasProcessed: true };
        }

        // Now parse SGR (color/style) sequences
        const ansiRegex = /\x1b\[([0-9;]*)m/g;
        let result = '';
        let lastIndex = 0;
        let currentClasses = [];
        let currentFgStyle = '';
        let currentBgStyle = '';

        let match;
        while ((match = ansiRegex.exec(text)) !== null) {
            // Add text before this escape sequence
            if (match.index > lastIndex) {
                const rawText = text.substring(lastIndex, match.index);

                // Apply color contrast adjustment if enabled
                let adjustedFgStyle = currentFgStyle;
                if (colorOffsetPercent > 0) {
                    const fgRgb = getFgRgb(currentClasses, currentFgStyle);
                    const bgRgb = getBgRgb(currentClasses, currentBgStyle);
                    const adjustedFg = adjustFgForContrast(fgRgb, bgRgb, colorOffsetPercent);
                    // Check if color was actually adjusted
                    if (adjustedFg[0] !== fgRgb[0] || adjustedFg[1] !== fgRgb[1] || adjustedFg[2] !== fgRgb[2]) {
                        adjustedFgStyle = `color:rgb(${adjustedFg[0]},${adjustedFg[1]},${adjustedFg[2]});`;
                    }
                }

                const classes = currentClasses.length > 0 ? ` class="${currentClasses.join(' ')}"` : '';
                const styles = (adjustedFgStyle || currentBgStyle) ? ` style="${adjustedFgStyle}${currentBgStyle}"` : '';

                // Check for shade characters that need blending
                const shadeResult = processShadeChars(rawText, currentClasses, currentFgStyle, currentBgStyle);
                if (shadeResult.wasProcessed) {
                    // Shade chars were processed, use the pre-built HTML
                    result += `<span${classes}${styles}>${shadeResult.processedHtml}</span>`;
                } else {
                    const textBefore = escapeHtml(rawText);
                    if (classes || styles) {
                        result += `<span${classes}${styles}>${textBefore}</span>`;
                    } else {
                        result += textBefore;
                    }
                }
            }

            // Parse the codes
            const codes = match[1].split(';').map(c => parseInt(c, 10) || 0);
            let i = 0;
            while (i < codes.length) {
                const code = codes[i];
                if (code === 0) {
                    // Reset all
                    currentClasses = [];
                    currentFgStyle = '';
                    currentBgStyle = '';
                } else if (code === 1) {
                    currentClasses.push('ansi-bold');
                    // Bold upgrades standard colors to bright variants
                    const stdColors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    for (const c of stdColors) {
                        const idx = currentClasses.indexOf('ansi-' + c);
                        if (idx !== -1) {
                            currentClasses[idx] = 'ansi-bright-' + c;
                            break;
                        }
                    }
                } else if (code === 3) {
                    currentClasses.push('ansi-italic');
                } else if (code === 4) {
                    currentClasses.push('ansi-underline');
                } else if (code === 5 || code === 6) {
                    currentClasses.push('ansi-blink');
                } else if (code >= 30 && code <= 37) {
                    // Basic foreground colors - use bright variant if bold is active
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
                    currentFgStyle = '';
                    const colors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    const isBold = currentClasses.includes('ansi-bold');
                    currentClasses.push((isBold ? 'ansi-bright-' : 'ansi-') + colors[code - 30]);
                } else if (code === 38) {
                    // Extended foreground color
                    if (codes[i + 1] === 5 && codes.length > i + 2) {
                        // 256-color mode: 38;5;N
                        const colorNum = codes[i + 2];
                        const rgb = color256ToRgb(colorNum);
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
                        currentFgStyle = `color:rgb(${rgb[0]},${rgb[1]},${rgb[2]});`;
                        i += 2;
                    } else if (codes[i + 1] === 2 && codes.length > i + 4) {
                        // True color mode: 38;2;R;G;B
                        const r = codes[i + 2];
                        const g = codes[i + 3];
                        const b = codes[i + 4];
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
                        currentFgStyle = `color:rgb(${r},${g},${b});`;
                        i += 4;
                    }
                } else if (code === 39) {
                    // Default foreground color
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
                    currentFgStyle = '';
                } else if (code >= 40 && code <= 47) {
                    // Basic background colors
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-bg-'));
                    currentBgStyle = '';
                    const colors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    currentClasses.push('ansi-bg-' + colors[code - 40]);
                } else if (code === 48) {
                    // Extended background color
                    if (codes[i + 1] === 5 && codes.length > i + 2) {
                        // 256-color mode: 48;5;N
                        const colorNum = codes[i + 2];
                        const rgb = color256ToRgb(colorNum);
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-bg-'));
                        currentBgStyle = `background-color:rgb(${rgb[0]},${rgb[1]},${rgb[2]});`;
                        i += 2;
                    } else if (codes[i + 1] === 2 && codes.length > i + 4) {
                        // True color mode: 48;2;R;G;B
                        const r = codes[i + 2];
                        const g = codes[i + 3];
                        const b = codes[i + 4];
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-bg-'));
                        currentBgStyle = `background-color:rgb(${r},${g},${b});`;
                        i += 4;
                    }
                } else if (code === 49) {
                    // Default background color
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-bg-'));
                    currentBgStyle = '';
                } else if (code >= 90 && code <= 97) {
                    // Bright foreground colors
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
                    currentFgStyle = '';
                    const colors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    currentClasses.push('ansi-bright-' + colors[code - 90]);
                } else if (code >= 100 && code <= 107) {
                    // Bright background colors
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-bg-'));
                    currentBgStyle = '';
                    const colors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    currentClasses.push('ansi-bg-bright-' + colors[code - 100]);
                }
                i++;
            }

            lastIndex = match.index + match[0].length;
        }

        // Add remaining text
        if (lastIndex < text.length) {
            const remaining = escapeHtml(text.substring(lastIndex));

            // Apply color contrast adjustment if enabled
            let adjustedFgStyle = currentFgStyle;
            if (colorOffsetPercent > 0) {
                const fgRgb = getFgRgb(currentClasses, currentFgStyle);
                const bgRgb = getBgRgb(currentClasses, currentBgStyle);
                const adjustedFg = adjustFgForContrast(fgRgb, bgRgb, colorOffsetPercent);
                // Check if color was actually adjusted
                if (adjustedFg[0] !== fgRgb[0] || adjustedFg[1] !== fgRgb[1] || adjustedFg[2] !== fgRgb[2]) {
                    adjustedFgStyle = `color:rgb(${adjustedFg[0]},${adjustedFg[1]},${adjustedFg[2]});`;
                }
            }

            const classes = currentClasses.length > 0 ? ` class="${currentClasses.join(' ')}"` : '';
            const styles = (adjustedFgStyle || currentBgStyle) ? ` style="${adjustedFgStyle}${currentBgStyle}"` : '';
            if (classes || styles) {
                result += `<span${classes}${styles}>${remaining}</span>`;
            } else {
                result += remaining;
            }
        }

        result = result || escapeHtml(text);

        // Final cleanup: strip any orphaned ANSI-like patterns that weren't matched
        // (e.g., [0m, [1;32m, [37m) - these appear when ESC char was lost
        // Negative lookahead prevents stripping [m from text like [match(, [menu], etc.
        result = result.replace(/\[([0-9;]*)m(?![a-zA-Z])/g, '');

        // Strip orphan ESC characters and the control picture symbol for ESC (␛ U+241B)
        // These can appear when ANSI sequences are incomplete or corrupted
        result = result.replace(/[\x1b\u001b\u241b]/g, '');

        return result;
    }

    // Convert Discord custom emoji tags to images
    // Format: <:name:id> or <a:name:id> (animated)
    function convertDiscordEmojis(html) {
        // Match Discord emoji format: <:name:id> or <a:name:id>
        return html.replace(/&lt;(a?):([^:]+):(\d+)&gt;/g, function(match, animated, name, id) {
            const ext = animated ? 'gif' : 'png';
            const url = `https://cdn.discordapp.com/emojis/${id}.${ext}`;
            return `<img src="${url}" alt=":${name}:" title=":${name}:" class="discord-emoji" style="height: 1.2em; vertical-align: middle;">`;
        });
    }

    // Escape HTML
    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    // Defense-in-depth: strip any event handler attributes from parsed HTML
    function sanitizeHtml(html) {
        return html.replace(/\bon\w+\s*=/gi, 'data-blocked=');
    }

    // Insert zero-width spaces after break characters in long words (>15 chars)
    // Break characters: [ ] ( ) , \ / - & = ? and spaces
    // Note: '.' is excluded because it breaks filenames (image.png) and domains awkwardly
    // Must be applied BEFORE parseAnsi (on raw text, not HTML)
    function insertWordBreaks(text) {
        const ZWSP = '\u200B'; // Zero-width space
        const BREAK_CHARS = [']', ')', ',', '\\', '/', '-', '_', '&', '=', '?', ';', ' '];
        const MIN_WORD_LEN = 15;

        let result = '';
        let wordLen = 0;
        let i = 0;

        while (i < text.length) {
            const c = text[i];
            result += c;
            i++;

            // Skip ANSI escape sequences entirely
            if (c === '\x1b' && text[i] === '[') {
                result += text[i++]; // consume '['
                // Consume until terminator (alphabetic or ~)
                while (i < text.length) {
                    const sc = text[i];
                    result += sc;
                    i++;
                    if ((sc >= 'A' && sc <= 'Z') || (sc >= 'a' && sc <= 'z') || sc === '~') {
                        break;
                    }
                }
                continue;
            }

            if (/\s/.test(c)) {
                wordLen = 0;
            } else {
                wordLen++;
                // Insert break opportunity after break chars in long words
                if (wordLen > MIN_WORD_LEN && BREAK_CHARS.includes(c)) {
                    result += ZWSP;
                }
            }
        }

        return result;
    }

    // Strip ANSI escape codes from text
    function stripAnsi(text) {
        if (!text) return text;
        // Remove all ANSI CSI sequences
        return text.replace(/\x1b\[[0-9;]*[A-Za-z@`~]/g, '').replace(/[\x00-\x1f]/g, '');
    }

    // Play ANSI music notes using Web Audio API
    // Uses square wave oscillator for authentic PC speaker sound
    function playAnsiMusic(notes) {
        if (!ansiMusicEnabled || !notes || notes.length === 0) return;

        // Lazily initialize AudioContext (requires user interaction in some browsers)
        if (!audioContext) {
            try {
                audioContext = new (window.AudioContext || window.webkitAudioContext)();
            } catch (e) {
                console.warn('Web Audio API not supported:', e);
                return;
            }
        }

        // Resume audio context if suspended (browser autoplay policy)
        if (audioContext.state === 'suspended') {
            audioContext.resume();
        }

        let startTime = audioContext.currentTime;

        notes.forEach(note => {
            if (note.frequency > 0) {
                // Create oscillator for this note
                const oscillator = audioContext.createOscillator();
                const gainNode = audioContext.createGain();

                oscillator.type = 'square';  // PC speaker sound
                oscillator.frequency.setValueAtTime(note.frequency, startTime);

                // Set volume (not too loud)
                gainNode.gain.setValueAtTime(0.15, startTime);

                // Quick fade out to avoid clicks
                const fadeTime = 0.01;
                const noteEnd = startTime + (note.duration_ms / 1000);
                gainNode.gain.setValueAtTime(0.15, noteEnd - fadeTime);
                gainNode.gain.linearRampToValueAtTime(0, noteEnd);

                oscillator.connect(gainNode);
                gainNode.connect(audioContext.destination);

                oscillator.start(startTime);
                oscillator.stop(noteEnd);
            }

            // Move start time forward for next note
            startTime += note.duration_ms / 1000;
        });
    }

    // ============================================================================
    // MCMP (MUD Client Media Protocol) - Media playback via GMCP
    // ============================================================================

    function ensureAudioContext() {
        if (!audioContext) {
            try {
                audioContext = new (window.AudioContext || window.webkitAudioContext)();
            } catch (e) {
                return false;
            }
        }
        if (audioContext.state === 'suspended') {
            audioContext.resume();
        }
        return true;
    }

    function handleMcmpMedia(action, dataStr, defaultUrl) {
        let data;
        try {
            data = JSON.parse(dataStr);
        } catch (e) {
            return;
        }

        switch (action) {
            case 'Default':
                if (data.url) {
                    mcmpDefaultUrl = data.url;
                }
                break;
            case 'Play':
                mcmpPlay(data, defaultUrl);
                break;
            case 'Stop':
                mcmpStop(data);
                break;
            case 'Load':
                mcmpLoad(data, defaultUrl);
                break;
        }
    }

    function mcmpResolveUrl(data, defaultUrl) {
        let baseUrl = data.url || mcmpDefaultUrl || defaultUrl || '';
        if (!baseUrl) return '';
        // Ensure base URL ends with /
        if (baseUrl && !baseUrl.endsWith('/')) baseUrl += '/';
        let name = data.name || '';
        if (!name) return baseUrl;
        // If name is already a full URL, use it directly
        if (name.startsWith('http://') || name.startsWith('https://')) return name;
        return baseUrl + name;
    }

    function mcmpPlay(data, defaultUrl) {
        let url = mcmpResolveUrl(data, defaultUrl);
        if (!url) return;

        let type = (data.type || 'sound').toLowerCase();
        let volume = data.volume !== undefined ? Math.max(0, Math.min(100, data.volume)) / 100 : 0.5;
        let loops = data.loops !== undefined ? data.loops : 1;
        let key = data.key || data.name || url;
        let continuePlay = data.continue !== undefined ? data.continue : true;

        if (type === 'music') {
            // Only one music track at a time
            if (mcmpMusicPlayer) {
                // If same file and continue:true, just adjust volume
                if (continuePlay && mcmpMusicPlayer.name === (data.name || url)) {
                    mcmpMusicPlayer.audio.volume = volume;
                    return;
                }
                // Stop current music
                mcmpStopAudio(mcmpMusicPlayer);
            }
            let audio = new Audio(url);
            audio.volume = volume;
            audio.loop = (loops === -1);
            if (loops > 1) {
                let playCount = 0;
                audio.addEventListener('ended', function() {
                    playCount++;
                    if (playCount < loops) {
                        audio.currentTime = 0;
                        audio.play().catch(() => {});
                    }
                });
            }
            audio.play().catch(() => {});
            mcmpMusicPlayer = { audio: audio, key: key, name: data.name || url };
        } else {
            // Sound - multiple simultaneous allowed
            let audio = new Audio(url);
            audio.volume = volume;
            audio.loop = (loops === -1);
            if (loops > 1) {
                let playCount = 0;
                audio.addEventListener('ended', function() {
                    playCount++;
                    if (playCount < loops) {
                        audio.currentTime = 0;
                        audio.play().catch(() => {});
                    } else {
                        delete mcmpSoundPlayers[key];
                    }
                });
            } else if (loops !== -1) {
                audio.addEventListener('ended', function() {
                    delete mcmpSoundPlayers[key];
                });
            }
            audio.play().catch(() => {});
            // Stop existing sound with same key
            if (mcmpSoundPlayers[key]) {
                mcmpStopAudio(mcmpSoundPlayers[key]);
            }
            mcmpSoundPlayers[key] = { audio: audio, key: key, name: data.name || url };
        }
    }

    function mcmpStop(data) {
        let type = data.type ? data.type.toLowerCase() : '';
        let key = data.key || '';
        let name = data.name || '';

        if (type === 'music' || (!type && !key && !name)) {
            // Stop music
            if (mcmpMusicPlayer) {
                mcmpStopAudio(mcmpMusicPlayer);
                mcmpMusicPlayer = null;
            }
        }
        if (type === 'sound' || (!type && !key && !name)) {
            // Stop all sounds
            for (let k in mcmpSoundPlayers) {
                mcmpStopAudio(mcmpSoundPlayers[k]);
            }
            mcmpSoundPlayers = {};
        }
        if (key) {
            // Stop by key
            if (mcmpMusicPlayer && mcmpMusicPlayer.key === key) {
                mcmpStopAudio(mcmpMusicPlayer);
                mcmpMusicPlayer = null;
            }
            if (mcmpSoundPlayers[key]) {
                mcmpStopAudio(mcmpSoundPlayers[key]);
                delete mcmpSoundPlayers[key];
            }
        }
        if (name && !key) {
            // Stop by name
            if (mcmpMusicPlayer && mcmpMusicPlayer.name === name) {
                mcmpStopAudio(mcmpMusicPlayer);
                mcmpMusicPlayer = null;
            }
            for (let k in mcmpSoundPlayers) {
                if (mcmpSoundPlayers[k].name === name) {
                    mcmpStopAudio(mcmpSoundPlayers[k]);
                    delete mcmpSoundPlayers[k];
                }
            }
        }
    }

    function mcmpStopAudio(player) {
        if (!player || !player.audio) return;
        player.audio.pause();
        player.audio.src = '';
    }

    function mcmpStopAll() {
        if (mcmpMusicPlayer) {
            mcmpStopAudio(mcmpMusicPlayer);
            mcmpMusicPlayer = null;
        }
        for (let k in mcmpSoundPlayers) {
            mcmpStopAudio(mcmpSoundPlayers[k]);
        }
        mcmpSoundPlayers = {};
    }

    function mcmpLoad(data, defaultUrl) {
        // Pre-cache by creating and immediately pausing
        let url = mcmpResolveUrl(data, defaultUrl);
        if (!url) return;
        let audio = new Audio(url);
        audio.preload = 'auto';
        audio.load();
    }

    // Linkify URLs in HTML text (after ANSI parsing)
    // Matches http://, https://, and www. URLs
    function linkifyUrls(html) {
        // URL pattern that works on HTML-escaped text
        // Matches http://, https://, or www. followed by non-whitespace
        // Stops at HTML tags, quotes, or common punctuation at end
        const urlPattern = /(\b(?:https?:\/\/|www\.)[^\s<>"'\u201C\u201D\u2018\u2019]*[^\s<>"'\u201C\u201D\u2018\u2019.,;:!?\)\]}>])/gi;

        return html.replace(urlPattern, function(url) {
            // Strip trailing HTML entities (complete like &quot; or partial like &quot
            // that got included because escapeHtml converts " to &quot; before this runs).
            // The regex stops at ; so we get partial entities like &quot or &amp at the end.
            let trimmed = url.replace(/&[a-zA-Z#0-9]*$/, '');
            const suffix = url.substring(trimmed.length);
            // Strip zero-width spaces from href (inserted by insertWordBreaks)
            const cleanUrl = trimmed.replace(/\u200B/g, '');
            // Add protocol if missing (for www. URLs)
            const href = cleanUrl.startsWith('www.') ? 'https://' + cleanUrl : cleanUrl;
            return `<a href="${href}" target="_blank" rel="noopener" class="output-link">${trimmed}</a>${suffix}`;
        });
    }

    // Format a timestamp for display
    // Returns "MM/DD HH:MM>" timestamp prefix
    function formatTimestamp(ts) {
        if (!ts) return '';

        // Convert seconds since epoch to Date
        const date = new Date(ts * 1000);

        const hours = date.getHours().toString().padStart(2, '0');
        const minutes = date.getMinutes().toString().padStart(2, '0');
        const day = date.getDate().toString().padStart(2, '0');
        const month = (date.getMonth() + 1).toString().padStart(2, '0');

        // Always show month/day with time
        return `${month}/${day} ${hours}:${minutes}> `;
    }

    // Convert a color name to CSS color value (for /highlight command)
    // Supports named colors, RGB values, and xterm 256-color codes
    function colorNameToCss(color) {
        if (!color || color.trim() === '') {
            return '#1a3a3a'; // Default dark cyan
        }
        const c = color.toLowerCase().trim();

        // Named colors (darker/muted for backgrounds)
        const namedColors = {
            'red': '#4a1515',
            'green': '#153a15',
            'blue': '#15153a',
            'yellow': '#3a3a15',
            'cyan': '#1a3a3a',
            'magenta': '#3a153a',
            'purple': '#3a153a',
            'orange': '#4a2a10',
            'pink': '#4a1530',
            'white': '#c0c0c0',
            'black': '#1a1a1a',
            'gray': '#3a3a3a',
            'grey': '#3a3a3a'
        };
        if (namedColors[c]) {
            return namedColors[c];
        }

        // Try xterm 256 color number
        const num = parseInt(c, 10);
        if (!isNaN(num) && num >= 0 && num <= 255) {
            return xterm256ToRgb(num);
        }

        // Try RGB format (r,g,b or r;g;b)
        const parts = c.includes(',') ? c.split(',') : c.split(';');
        if (parts.length === 3) {
            const r = parseInt(parts[0].trim(), 10);
            const g = parseInt(parts[1].trim(), 10);
            const b = parseInt(parts[2].trim(), 10);
            if (!isNaN(r) && !isNaN(g) && !isNaN(b)) {
                return `rgb(${r}, ${g}, ${b})`;
            }
        }

        return '#1a3a3a'; // Default fallback
    }

    // Convert xterm 256 color code to RGB hex
    function xterm256ToRgb(code) {
        // Standard colors (0-15) - return muted versions
        const standard = [
            '#000000', '#800000', '#008000', '#808000', '#000080', '#800080', '#008080', '#c0c0c0',
            '#808080', '#ff0000', '#00ff00', '#ffff00', '#0000ff', '#ff00ff', '#00ffff', '#ffffff'
        ];
        if (code < 16) {
            return standard[code];
        }
        // 216 color cube (16-231)
        if (code < 232) {
            const c = code - 16;
            const r = Math.floor(c / 36) * 51;
            const g = Math.floor((c % 36) / 6) * 51;
            const b = (c % 6) * 51;
            return `rgb(${r}, ${g}, ${b})`;
        }
        // Grayscale (232-255)
        const gray = (code - 232) * 10 + 8;
        return `rgb(${gray}, ${gray}, ${gray})`;
    }

    // Strip MUD tags like [channel:] or [channel(player)] from start of line
    // Preserves leading whitespace and ANSI codes
    function stripMudTag(text) {
        if (!text) return text;

        // Find leading whitespace
        const trimmed = text.trimStart();
        const leadingWsLen = text.length - trimmed.length;
        const leadingWs = text.substring(0, leadingWsLen);

        // Don't strip tags from indented lines - real MUD tags are never indented
        if (leadingWsLen > 0) return text;

        // Parse through ANSI codes to find actual content start
        let i = 0;
        let ansiPrefix = '';
        let inAnsi = false;

        while (i < trimmed.length) {
            const c = trimmed[i];
            if (c === '\x1b' && trimmed[i + 1] === '[') {
                ansiPrefix += c;
                inAnsi = true;
                i++;
            } else if (inAnsi) {
                ansiPrefix += c;
                if (/[a-zA-Z]/.test(c)) {
                    inAnsi = false;
                }
                i++;
            } else if (c === '[') {
                // Found start of potential tag
                const rest = trimmed.substring(i + 1);
                const endBracket = rest.indexOf(']');
                if (endBracket >= 0) {
                    const tag = rest.substring(0, endBracket);
                    // Match two specific MUD tag patterns:
                    //   [name(content)optional] - paren group inside brackets
                    //   [name:] - colon immediately before closing bracket
                    const parenStart = tag.indexOf('(');
                    let isTag;
                    if (parenStart > 0) {
                        // Pattern 1: [name(content)optional] - non-empty content inside parens
                        const parenEnd = tag.indexOf(')', parenStart);
                        isTag = parenEnd > parenStart + 1;
                    } else {
                        // Pattern 2: [name:] - colon at end with content before it
                        isTag = tag.length > 1 && tag.endsWith(':');
                    }
                    if (isTag) {
                        // Require a space after '] ' (matching Perl patterns)
                        const afterTag = rest.substring(endBracket + 1);
                        if (afterTag.startsWith(' ')) {
                            return leadingWs + ansiPrefix + afterTag.substring(1);
                        }
                    }
                }
                // Not a MUD tag, return original
                return text;
            } else {
                // Not a tag start, return original
                return text;
            }
        }

        return text;
    }

    // Convert temperatures: "32C" -> "32C (90F)", "100F" -> "100F (38C)"
    function convertTemperatures(text) {
        if (!text) return text;
        // Pattern: number (with optional decimal), optional space, C or F, followed by delimiter or end
        return text.replace(/(-?\d+(?:\.\d+)?)\s?([CcFf])([\s.,;:!?\]\)"']|$)/g, (match, num, unit, delim) => {
            const n = parseFloat(num);
            if (isNaN(n)) return match;
            let converted, newUnit;
            if (unit.toUpperCase() === 'C') {
                // Celsius to Fahrenheit: (C * 9/5) + 32
                converted = Math.round((n * 9 / 5) + 32);
                newUnit = 'F';
            } else {
                // Fahrenheit to Celsius: (F - 32) * 5/9
                converted = Math.round((n - 32) * 5 / 9);
                newUnit = 'C';
            }
            return `${num}${match.includes(' ' + unit) ? ' ' : ''}${unit} (${converted}${newUnit})${delim}`;
        });
    }

    // Scroll to bottom
    function scrollToBottom() {
        elements.outputContainer.scrollTop = elements.outputContainer.scrollHeight;
    }

    // Batched scroll-to-bottom via requestAnimationFrame (avoids forced layout per line)
    let scrollRafPending = false;
    function scheduleScrollToBottom() {
        if (!scrollRafPending) {
            scrollRafPending = true;
            requestAnimationFrame(() => {
                scrollRafPending = false;
                scrollToBottom();
            });
        }
    }

    // Format count for status indicator (right-justified, 4 chars)
    function formatCount(n) {
        if (n >= 1000000) return 'Alot';
        if (n >= 10000) return (Math.min(Math.floor(n / 1000), 999) + 'K').padStart(4, ' ');
        return n.toString().padStart(4, ' ');
    }

    // Update status bar
    function updateStatusBar() {
        const world = worlds[currentWorldIndex];

        // Connection dot and world name
        if (world && world.name && world.was_connected) {
            elements.statusDot.className = 'status-dot' + (world.connected ? '' : ' off');
            const gmcpInd = (world && world.gmcp_user_enabled) ? ' [g]' : '';
            elements.worldName.textContent = world.name + gmcpInd;
            elements.statusDot.style.display = '';
            elements.worldName.style.display = '';
        } else {
            elements.statusDot.style.display = 'none';
            elements.worldName.style.display = 'none';
        }

        // More/Hist badge
        const serverPending = world ? (world.pending_count || 0) : 0;
        const totalPending = pendingLines.length + serverPending;
        if (!isAtBottom() && !scrollRafPending) {
            const container = elements.outputContainer;
            const fontSize = currentFontSize || 14;
            const lineHeight = fontSize * 1.2;
            const linesFromBottom = Math.floor((container.scrollHeight - container.scrollTop - container.clientHeight) / lineHeight);
            elements.moreLabel.textContent = 'History';
            elements.moreCount.textContent = formatCount(linesFromBottom);
            elements.statusMore.style.display = '';
        } else if ((paused && pendingLines.length > 0) || serverPending > 0) {
            elements.moreLabel.textContent = 'More';
            elements.moreCount.textContent = formatCount(totalPending);
            elements.statusMore.style.display = '';
        } else {
            elements.statusMore.style.display = 'none';
        }

        // Activity badge with hover tooltip showing which worlds have activity
        if (serverActivityCount > 0) {
            elements.activityCount.textContent = serverActivityCount;
            elements.activityIndicator.style.display = '';
            // Build tooltip listing worlds with activity
            const activeWorlds = worlds
                .filter((w, i) => i !== currentWorldIndex && ((w.unseen_lines || 0) > 0 || (w.pending_count || 0) > 0))
                .map(w => w.name);
            elements.activityIndicator.title = activeWorlds.length > 0
                ? 'Unseen: ' + activeWorlds.join(', ')
                : '';
        } else {
            elements.activityIndicator.style.display = 'none';
            elements.activityIndicator.title = '';
        }
    }

    // Update time (12-hour format H:MM, no AM/PM)
    function updateTime() {
        const now = new Date();
        let hours = now.getHours() % 12;
        if (hours === 0) hours = 12;
        const minutes = now.getMinutes().toString().padStart(2, '0');
        elements.statusTime.textContent = `${hours}:${minutes}`;
    }

    // Set input area height (number of lines)
    function setInputHeight(lines) {
        inputHeight = Math.max(1, Math.min(15, lines));
        const fontSize = currentFontSize || 14;
        const lineHeight = 1.2 * fontSize; // line-height * font-size
        elements.input.style.height = (inputHeight * lineHeight) + 'px';
        elements.input.rows = inputHeight;
    }

    // Force browser to repaint (fixes delayed rendering when tab isn't focused)
    function forceRepaint(element) {
        void element.offsetHeight;
    }

    // Show/hide connecting overlay
    function showConnecting(show) {
        elements.connectingOverlay.className = 'overlay' + (show ? ' visible' : '');
        forceRepaint(elements.connectingOverlay);
    }

    // Show/hide connection error modal
    function showConnectionErrorModal() {
        elements.connectionErrorText.textContent = 'Unable to connect to the server.';
        elements.connectionErrorModal.className = 'modal visible';
        elements.connectionErrorModal.style.display = 'flex';
        forceRepaint(elements.connectionErrorModal);
    }

    function hideConnectionErrorModal() {
        elements.connectionErrorModal.className = 'modal';
        elements.connectionErrorModal.style.display = 'none';
    }

    // Show/hide reconnect modal
    function showReconnectModal() {
        elements.reconnectModal.className = 'modal visible';
        elements.reconnectModal.style.display = 'flex';
        forceRepaint(elements.reconnectModal);
    }

    function hideReconnectModal() {
        elements.reconnectModal.className = 'modal';
        elements.reconnectModal.style.display = 'none';
    }

    // Show/hide auth modal
    function showAuthModal(show) {
        elements.authModal.className = 'modal' + (show ? ' visible' : '');
        forceRepaint(elements.authModal);
        if (show) {
            // Hide all UI elements when showing auth modal
            elements.output.innerHTML = '';
            if (elements.statusBar) elements.statusBar.style.display = 'none';
            if (elements.navBar) elements.navBar.style.display = 'none';
            if (elements.inputContainer) elements.inputContainer.style.display = 'none';
            if (elements.outputContainer) elements.outputContainer.style.display = 'none';
            // Close any open menus
            closeMenu();
            elements.authPassword.value = '';
            elements.authError.textContent = '';
            if (elements.authUsername) {
                elements.authUsername.value = '';
            }
            // Show auth key field on Android so user can see/edit it
            if (elements.authKeyRow && elements.authKeyInput) {
                if (window.Android) {
                    elements.authKeyRow.style.display = '';
                    elements.authKeyInput.value = authKey || '';
                } else {
                    elements.authKeyRow.style.display = 'none';
                }
            }
        } else {
            // Restore UI elements when hiding auth modal
            setupToolbars(deviceMode);
            if (elements.statusBar) elements.statusBar.style.display = '';
            if (elements.navBar) elements.navBar.style.display = '';
            if (elements.inputContainer) elements.inputContainer.style.display = '';
            if (elements.outputContainer) elements.outputContainer.style.display = '';
        }
    }

    // Show/hide password change modal (multiuser mode only)
    function showPasswordModal(show) {
        if (!elements.passwordModal) return;
        elements.passwordModal.className = 'modal' + (show ? ' visible' : '');
        forceRepaint(elements.passwordModal);
        if (show) {
            elements.passwordOld.value = '';
            elements.passwordNew.value = '';
            elements.passwordConfirm.value = '';
            elements.passwordError.textContent = '';
            elements.passwordOld.focus();
        }
    }

    // Update UI based on Android app detection
    function updateAndroidUI() {
        const isAndroid = typeof Android !== 'undefined' && Android.openServerSettings;
        // Show Clay Server settings tab button only in Android app
        const clayServerTabBtn = document.getElementById('settings-clay-server-btn');
        if (clayServerTabBtn) clayServerTabBtn.style.display = isAndroid ? '' : 'none';
        // Show auth key Download button only in Android app (starts disabled until connected)
        const dlBtn = document.getElementById('cs-auth-key-download');
        if (dlBtn) {
            dlBtn.style.display = isAndroid ? '' : 'none';
            if (isAndroid) { dlBtn.disabled = true; dlBtn.style.opacity = '0.4'; }
        }
        // Show Reload menu item only in WebView GUI mode (not pure web)
        document.querySelectorAll('.menu-reload').forEach(el => {
            el.style.display = window.WEBVIEW_MODE ? '' : 'none';
        });
        // Hide Resync for master WebView GUI (it IS the master, resync is meaningless)
        if (window.WEBVIEW_MODE && window.AUTO_PASSWORD) {
            document.querySelectorAll('.menu-resync').forEach(el => {
                el.style.display = 'none';
            });
        }
    }

    // Populate the Clay Server settings tab fields from Android SharedPreferences
    function populateClayServerTab() {
        if (!window.Android || typeof window.Android.getConnectionInfo !== 'function') return;
        try {
            var info = JSON.parse(window.Android.getConnectionInfo());
            var hostEl = document.getElementById('cs-host');
            var portEl = document.getElementById('cs-port');
            var remoteEl = document.getElementById('cs-remote-host');
            var userEl = document.getElementById('cs-username');
            var passEl = document.getElementById('cs-password');
            var keyEl = document.getElementById('cs-auth-key');
            if (hostEl) hostEl.value = info.localHost || '';
            if (portEl) portEl.value = info.port || 9000;
            if (remoteEl) remoteEl.value = info.remoteHost || '';
            if (userEl) userEl.value = (typeof window.Android.getSavedUsername === 'function') ? window.Android.getSavedUsername() : '';
            if (passEl) passEl.value = (typeof window.Android.getSavedPassword === 'function') ? window.Android.getSavedPassword() : '';
            if (keyEl) keyEl.value = '';  // never pre-populate; user clicks Download to store it
            // Enable download button only when connected (live key available from server)
            var dlBtn = document.getElementById('cs-auth-key-download');
            if (dlBtn) {
                var hasKey = !!serverAuthKey;
                dlBtn.disabled = !hasKey;
                dlBtn.style.opacity = hasKey ? '' : '0.4';
                dlBtn.title = hasKey ? 'Save key from server into app' : 'Connect to server first';
                dlBtn.textContent = 'Download';
            }
        } catch(e) {}
    }

    // Update UI based on multiuser mode
    function updateMultiuserUI() {
        // Show/hide change password menu item
        document.querySelectorAll('.menu-change-password').forEach(el => {
            el.style.display = multiuserMode ? '' : 'none';
        });

        // Show/hide logout menu item and its divider
        document.querySelectorAll('.menu-logout').forEach(el => {
            el.style.display = multiuserMode ? '' : 'none';
        });
        document.querySelectorAll('.menu-logout-divider').forEach(el => {
            el.style.display = multiuserMode ? '' : 'none';
        });

        // In multiuser mode, hide world editor buttons (Add, Edit, Delete)
        if (multiuserMode) {
            if (elements.worldAddBtn) elements.worldAddBtn.style.display = 'none';
            if (elements.worldEditBtn) elements.worldEditBtn.style.display = 'none';
            if (elements.worldEditDeleteBtn) elements.worldEditDeleteBtn.style.display = 'none';

            // Hide web settings menu item
            document.querySelectorAll('[data-action="web"]').forEach(el => {
                el.style.display = 'none';
            });
        }
    }

    // Enable multiuser mode UI (show username field in auth modal)
    function enableMultiuserAuthUI() {
        multiuserMode = true;
        if (elements.authUsernameRow) {
            elements.authUsernameRow.style.display = '';
        }
        if (elements.authPrompt) {
            elements.authPrompt.textContent = 'Enter your username and password:';
        }
        if (elements.authUsername) {
            elements.authUsername.focus();
        }
    }

    // Actions popup functions (split into List and Editor)

    // Open Actions List popup
    function openActionsListPopup(worldFilter = null) {
        actionsListPopupOpen = true;
        actionsWorldFilter = worldFilter || '';
        elements.actionFilter.value = '';
        elements.actionWorldFilterIndicator.textContent = worldFilter ? `World: ${worldFilter}` : '';
        selectedActionIndex = -1;
        elements.actionsListModal.className = 'modal visible';
        renderActionsList();
        // Select first visible action
        const firstVisible = getFilteredActionIndices()[0];
        if (firstVisible !== undefined) {
            selectedActionIndex = firstVisible;
            renderActionsList();
        }
        elements.actionFilter.focus();
    }

    // Close Actions List popup
    function closeActionsListPopup() {
        actionsListPopupOpen = false;
        actionsWorldFilter = '';
        elements.actionFilter.value = '';
        elements.actionWorldFilterIndicator.textContent = '';
        elements.actionsListModal.className = 'modal';
        elements.input.focus();
    }

    // Get indices of actions matching current filters
    function getFilteredActionIndices() {
        const filterText = elements.actionFilter.value.toLowerCase();
        const worldFilterLower = actionsWorldFilter.toLowerCase();

        return actions
            .map((action, index) => ({ action, index }))
            .filter(({ action }) => {
                // World filter (from /actions <world>)
                if (worldFilterLower && !action.world.toLowerCase().includes(worldFilterLower)) {
                    return false;
                }
                // Text filter (from filter input)
                if (filterText) {
                    const nameMatch = action.name.toLowerCase().includes(filterText);
                    const worldMatch = action.world.toLowerCase().includes(filterText);
                    const patternMatch = action.pattern.toLowerCase().includes(filterText);
                    if (!nameMatch && !worldMatch && !patternMatch) {
                        return false;
                    }
                }
                return true;
            })
            .sort((a, b) => a.action.name.toLowerCase().localeCompare(b.action.name.toLowerCase()))
            .map(({ index }) => index);
    }

    // Render actions list with Name, World, Pattern columns
    function renderActionsList() {
        elements.actionsList.innerHTML = '';
        const filteredIndices = getFilteredActionIndices();

        // Dynamically size the list to show all actions without overlapping separator/input
        // Each item is approximately 26px (padding + content + border)
        const itemHeight = 26;
        const minHeight = 80;  // At least show a few items
        // Calculate available height: window height minus status bar, nav bar, input, and popup chrome
        const statusBarHeight = elements.statusBar ? elements.statusBar.offsetHeight : 26;
        const navBarHeight = elements.navBar ? elements.navBar.offsetHeight : 0;
        const inputContainerHeight = elements.inputContainer ? elements.inputContainer.offsetHeight : 80;
        const popupChrome = 180; // Approximate space for popup header, filter, buttons, margins
        const maxAvailable = window.innerHeight - statusBarHeight - navBarHeight - inputContainerHeight - popupChrome;
        // Height needed to show all filtered items
        const neededHeight = filteredIndices.length * itemHeight;
        // Use the smaller of needed or available, but at least minHeight
        const listHeight = Math.max(minHeight, Math.min(neededHeight, maxAvailable));
        elements.actionsList.style.maxHeight = listHeight + 'px';
        elements.actionsList.style.minHeight = minHeight + 'px';

        if (actions.length === 0) {
            const div = document.createElement('div');
            div.style.padding = '8px';
            div.style.color = '#888';
            div.textContent = 'No actions defined.';
            elements.actionsList.appendChild(div);
            return;
        }

        if (filteredIndices.length === 0) {
            const div = document.createElement('div');
            div.style.padding = '8px';
            div.style.color = '#888';
            div.textContent = 'No matching actions.';
            elements.actionsList.appendChild(div);
            return;
        }

        // Add header row
        const headerDiv = document.createElement('div');
        headerDiv.className = 'actions-list-header';
        const nameHeader = document.createElement('span');
        nameHeader.className = 'action-name';
        nameHeader.textContent = 'Name';
        headerDiv.appendChild(nameHeader);
        const worldHeader = document.createElement('span');
        worldHeader.className = 'action-world';
        worldHeader.textContent = 'World';
        headerDiv.appendChild(worldHeader);
        const patternHeader = document.createElement('span');
        patternHeader.className = 'action-pattern';
        patternHeader.textContent = 'Pattern';
        headerDiv.appendChild(patternHeader);
        elements.actionsList.appendChild(headerDiv);

        filteredIndices.forEach((index) => {
            const action = actions[index];
            const div = document.createElement('div');
            div.className = 'actions-list-item' + (index === selectedActionIndex ? ' selected' : '');

            const nameSpan = document.createElement('span');
            nameSpan.className = 'action-name';
            nameSpan.textContent = action.name || '(unnamed)';
            div.appendChild(nameSpan);

            const worldSpan = document.createElement('span');
            worldSpan.className = 'action-world';
            worldSpan.textContent = action.world || '(all)';
            div.appendChild(worldSpan);

            const patternSpan = document.createElement('span');
            patternSpan.className = 'action-pattern';
            patternSpan.textContent = action.pattern || '(manual)';
            div.appendChild(patternSpan);

            div.onclick = () => {
                selectedActionIndex = index;
                renderActionsList();
            };
            div.ondblclick = () => {
                selectedActionIndex = index;
                openActionsEditorPopup(index);
            };
            elements.actionsList.appendChild(div);
        });
    }

    // Open Actions Editor popup
    function openActionsEditorPopup(editIndex) {
        actionsEditorPopupOpen = true;
        editingActionIndex = editIndex;
        elements.actionsListModal.className = 'modal';  // Hide list
        elements.actionsEditorModal.className = 'modal visible';

        if (editIndex >= 0 && editIndex < actions.length) {
            // Editing existing action
            elements.actionEditorTitle.textContent = 'Edit Action';
            const action = actions[editIndex];
            elements.actionName.value = action.name || '';
            elements.actionWorld.value = action.world || '';
            const matchType = action.match_type === 'Wildcard' ? 'wildcard' : 'regexp';
            elements.actionMatchType.value = matchType;
            elements.actionPattern.placeholder = matchType === 'wildcard'
                ? '(wildcard: * and ?, empty = manual only)'
                : '(regex, empty = manual only)';
            elements.actionPattern.value = action.pattern || '';
            elements.actionCommand.value = action.command || '';
            // Default to true if enabled is not set (for existing actions)
            elements.actionEnabled.value = (action.enabled !== false) ? 'yes' : 'no';
            elements.actionStartup.value = action.startup ? 'yes' : 'no';
        } else {
            // New action
            elements.actionEditorTitle.textContent = 'New Action';
            elements.actionName.value = '';
            elements.actionWorld.value = '';
            elements.actionMatchType.value = 'regexp';  // Default to Regexp
            elements.actionPattern.placeholder = '(regex, empty = manual only)';
            elements.actionPattern.value = '';
            elements.actionCommand.value = '';
            elements.actionEnabled.value = 'yes';  // Default to enabled
            elements.actionStartup.value = 'no';  // Default to disabled
        }
        elements.actionError.textContent = '';
        elements.actionEditorDeleteBtn.style.display = (editIndex >= 0) ? '' : 'none';
        elements.actionName.focus();
    }

    // Close Actions Editor popup (return to list)
    function closeActionsEditorPopup() {
        actionsEditorPopupOpen = false;
        elements.actionsEditorModal.className = 'modal';
        elements.actionsListModal.className = 'modal visible';
        actionsListPopupOpen = true;
        renderActionsList();
    }

    // Open delete confirmation popup
    function openActionsConfirmPopup() {
        if (selectedActionIndex < 0 || selectedActionIndex >= actions.length) return;
        actionsConfirmPopupOpen = true;
        const actionName = actions[selectedActionIndex].name || '(unnamed)';
        elements.actionConfirmText.textContent = `Delete action '${actionName}'?`;
        elements.actionConfirmModal.className = 'modal visible';
    }

    // Close delete confirmation popup
    function closeActionsConfirmPopup() {
        actionsConfirmPopupOpen = false;
        elements.actionConfirmModal.className = 'modal';
    }

    // Confirm delete action
    function confirmDeleteAction() {
        if (selectedActionIndex >= 0 && selectedActionIndex < actions.length) {
            actions.splice(selectedActionIndex, 1);
            if (selectedActionIndex >= actions.length) {
                selectedActionIndex = actions.length - 1;
            }
            // Send to server
            send({
                type: 'UpdateActions',
                actions: actions
            });
            renderActionsList();
        }
        closeActionsConfirmPopup();
        // If editor was open (delete from editor), close it and return to list
        if (actionsEditorPopupOpen) {
            closeActionsEditorPopup();
        }
    }

    function validateAction(name, editIndex) {
        if (!name) {
            return 'Name is required';
        }
        // Check for duplicate names (excluding current if editing)
        const duplicateIndex = actions.findIndex((a, i) =>
            a.name.toLowerCase() === name.toLowerCase() && i !== editIndex
        );
        if (duplicateIndex >= 0) {
            return 'An action with this name already exists';
        }
        // Check for internal command conflicts
        const internalCommands = ['help', 'disconnect', 'dc', 'setup', 'world', 'worlds', 'l', 'reload', 'quit', 'actions', 'gag'];
        if (internalCommands.includes(name.toLowerCase())) {
            return 'Cannot use internal command name';
        }
        return null;
    }

    function saveAction() {
        const name = elements.actionName.value.trim();
        const error = validateAction(name, editingActionIndex);
        if (error) {
            elements.actionError.textContent = error;
            return;
        }

        const actionData = {
            name: name,
            world: elements.actionWorld.value.trim(),
            match_type: elements.actionMatchType.value === 'wildcard' ? 'Wildcard' : 'Regexp',
            pattern: elements.actionPattern.value,
            command: elements.actionCommand.value,
            enabled: elements.actionEnabled.value === 'yes',
            startup: elements.actionStartup.value === 'yes'
        };

        if (editingActionIndex < 0) {
            // New action
            actions.push(actionData);
            selectedActionIndex = actions.length - 1;
        } else {
            // Update existing
            actions[editingActionIndex] = actionData;
        }

        // Send to server
        send({
            type: 'UpdateActions',
            actions: actions
        });

        closeActionsEditorPopup();
    }

    // Legacy function for compatibility
    function openActionsPopup() {
        openActionsListPopup();
    }

    function closeActionsPopup() {
        if (actionsEditorPopupOpen) {
            closeActionsEditorPopup();
        } else if (actionsConfirmPopupOpen) {
            closeActionsConfirmPopup();
        } else {
            closeActionsListPopup();
        }
    }

    // Combined settings popup functions (/setup + /web)
    function switchSettingsTab(tab) {
        settingsActiveTab = tab;
        document.querySelectorAll('.settings-tab-btn').forEach(function(btn) {
            btn.classList.toggle('active', btn.dataset.tab === tab);
        });
        elements.settingsGeneralSection.classList.toggle('active', tab === 'general');
        elements.settingsWebSection.classList.toggle('active', tab === 'web');
        elements.settingsFontSection.classList.toggle('active', tab === 'font');
        if (elements.settingsClayServerSection) {
            elements.settingsClayServerSection.classList.toggle('active', tab === 'clay-server');
        }
        var titles = { general: 'General', web: 'Web', font: 'Font', 'clay-server': 'Clay Server' };
        elements.settingsTitle.textContent = titles[tab] || tab;
        // Rename Save button when on clay-server tab (will reconnect)
        if (elements.settingsSaveBtn) {
            elements.settingsSaveBtn.textContent = tab === 'clay-server' ? 'Save & Connect' : 'Save';
        }
        if (tab === 'clay-server') {
            populateClayServerTab();
        }
    }

    function openEditorPage(page) {
        var url;
        if (window.SERVER_URL) {
            url = window.SERVER_URL + '/' + page;
        } else {
            var proto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
            var host = window.WS_HOST || window.location.hostname;
            var port = (window.WS_PORT && window.WS_PORT !== 0) ? window.WS_PORT : window.location.port;
            url = proto + '://' + host + ':' + port + '/' + page;
        }
        if (window.WEBVIEW_MODE && window.ipc) {
            window.ipc.postMessage('open-url:' + url);
        } else {
            window.open(url, '_blank');
        }
    }

    function openSettingsPopup(tab) {
        if (tab === 'web' && multiuserMode) {
            appendClientLine('Web settings are disabled in multiuser mode.', currentWorldIndex, 'system');
            return;
        }
        settingsPopupOpen = true;
        // Load general edit state
        setupMoreMode = moreModeEnabled;
        setupWorldSwitchMode = worldSwitchMode;
        setupAnsiMusic = ansiMusicEnabled;
        setupZwj = zwjEnabled;
        setupTtsMode = ttsMode === 'off' ? 'Off' : ttsMode === 'local' ? 'Local' : ttsMode === 'edge' ? 'Edge' : 'Off';
        setupTlsProxy = tlsProxyEnabled;
        setupNewLineIndicator = newLineIndicator;
        setupDebug = debugEnabled;
        setupInputHeightValue = inputHeight;
        setupGuiTheme = guiTheme;
        setupColorOffset = colorOffsetPercent;
        setupTransparency = guiTransparency;
        // Load web edit state
        editWebSecure = webSecure;
        editHttpEnabled = httpEnabled;
        // Load font edit state
        fontEditName = fontName;
        fontEditSizePhone = Math.round(webFontSizePhone);
        fontEditSizeTablet = Math.round(webFontSizeTablet);
        fontEditSizeDesktop = Math.round(webFontSizeDesktop);
        fontEditWeight = webFontWeight;
        fontEditLineHeight = webFontLineHeight;
        fontEditLetterSpacing = webFontLetterSpacing;
        fontEditWordSpacing = webFontWordSpacing;
        // Set advanced checkbox based on whether any advanced setting is non-default
        if (elements.fontAdvancedToggle) {
            elements.fontAdvancedToggle.checked = (webFontLineHeight !== 1.2 || webFontLetterSpacing !== 0 || webFontWordSpacing !== 0);
        }
        // Show modal
        elements.settingsModal.className = 'modal visible';
        elements.settingsModal.style.display = 'flex';
        switchSettingsTab(tab || 'general');
        updateSetupPopupUI();
        updateWebPopupUI();
        renderFontFamilyList();
        updateFontPopupUI();
    }

    function closeSettingsPopup() {
        settingsPopupOpen = false;
        elements.settingsModal.className = 'modal';
        elements.settingsModal.style.display = 'none';
        focusInputWithKeyboard();
    }

    function updateSetupPopupUI() {
        // Toggle switches
        if (setupMoreMode) {
            elements.setupMoreModeToggle.classList.add('active');
        } else {
            elements.setupMoreModeToggle.classList.remove('active');
        }
        // Note: show tags removed from setup - controlled by F2 or /tag command
        if (setupAnsiMusic) {
            elements.setupAnsiMusicToggle.classList.add('active');
        } else {
            elements.setupAnsiMusicToggle.classList.remove('active');
        }
        if (setupZwj) {
            elements.setupZwjToggle.classList.add('active');
        } else {
            elements.setupZwjToggle.classList.remove('active');
        }
        elements.setupTtsSelect.value = setupTtsMode;
        updateCustomDropdown(elements.setupTtsSelect);
        if (elements.setupTtsSpeakModeSelect) {
            elements.setupTtsSpeakModeSelect.value = ttsSpeakMode;
            updateCustomDropdown(elements.setupTtsSpeakModeSelect);
        }
        if (setupTlsProxy) {
            elements.setupTlsProxyToggle.classList.add('active');
        } else {
            elements.setupTlsProxyToggle.classList.remove('active');
        }
        if (setupNewLineIndicator) {
            elements.setupNewLineIndicatorToggle.classList.add('active');
        } else {
            elements.setupNewLineIndicatorToggle.classList.remove('active');
        }
        if (setupDebug) {
            elements.setupDebugToggle.classList.add('active');
        } else {
            elements.setupDebugToggle.classList.remove('active');
        }
        // World switching dropdown
        elements.setupWorldSwitchSelect.value = setupWorldSwitchMode;
        updateCustomDropdown(elements.setupWorldSwitchSelect);
        // Input height stepper
        elements.setupInputHeightValue.textContent = setupInputHeightValue;
        // Color offset stepper
        elements.setupColorOffsetValue.textContent = setupColorOffset === 0 ? 'OFF' : setupColorOffset + '%';
        // Theme dropdown
        elements.setupThemeSelect.value = setupGuiTheme.charAt(0).toUpperCase() + setupGuiTheme.slice(1);
        updateCustomDropdown(elements.setupThemeSelect);
        // Transparency slider (webview mode only)
        if (window.WEBVIEW_MODE && elements.setupTransparencyRow) {
            elements.setupTransparencyRow.style.display = '';
            elements.setupTransparencySlider.value = Math.round(setupTransparency * 100);
            elements.setupTransparencyValue.textContent = Math.round(setupTransparency * 100) + '%';
        }
    }

    // Build an UpdateGlobalSettings message with current state
    function buildUpdateGlobalSettings() {
        return {
            type: 'UpdateGlobalSettings',
            more_mode_enabled: moreModeEnabled,
            spell_check_enabled: spellCheckEnabled,
            temp_convert_enabled: tempConvertEnabled,
            world_switch_mode: worldSwitchMode,
            show_tags: showTags,
            ansi_music_enabled: ansiMusicEnabled,
            input_height: inputHeight,
            console_theme: consoleTheme,
            gui_theme: guiTheme,
            gui_transparency: guiTransparency,
            color_offset_percent: colorOffsetPercent,
            font_name: fontName,
            font_size: guiFontSize,
            web_font_size_phone: webFontSizePhone,
            web_font_size_tablet: webFontSizeTablet,
            web_font_size_desktop: webFontSizeDesktop,
            web_font_weight: webFontWeight,
            web_font_line_height: webFontLineHeight,
            web_font_letter_spacing: webFontLetterSpacing,
            web_font_word_spacing: webFontWordSpacing,
            ws_allow_list: wsAllowList,
            web_secure: webSecure,
            http_enabled: httpEnabled,
            http_port: httpPort,
            ws_enabled: wsEnabled,
            ws_port: wsPort,
            ws_cert_file: wsCertFile,
            ws_key_file: wsKeyFile,
            ws_password: wsPassword,
            tls_proxy_enabled: tlsProxyEnabled,
            zwj_enabled: zwjEnabled,
            tts_mode: ttsMode,
            tts_speak_mode: ttsSpeakMode,
            new_line_indicator: newLineIndicator,
            mouse_enabled: mouseEnabled,
            debug_enabled: debugEnabled,
            dictionary_path: dictionaryPath
        };
    }

    function saveSettingsAll() {
        // Clay Server tab (Android only) — save to SharedPreferences and reload
        if (settingsActiveTab === 'clay-server' && window.Android) {
            var host = (document.getElementById('cs-host') || {}).value || '';
            host = host.trim();
            if (!host) {
                var statusEl = document.getElementById('cs-host');
                if (statusEl) { statusEl.focus(); statusEl.style.outline = '2px solid var(--theme-error)'; }
                return;
            }
            var port = ((document.getElementById('cs-port') || {}).value || '9000').trim();
            var remoteHost = ((document.getElementById('cs-remote-host') || {}).value || '').trim();
            var username = ((document.getElementById('cs-username') || {}).value || '').trim();
            var password = (document.getElementById('cs-password') || {}).value || '';
            var authKey = ((document.getElementById('cs-auth-key') || {}).value || '').trim();
            if (typeof window.Android.saveConnectionSettings === 'function') {
                window.Android.saveConnectionSettings(host, port, remoteHost);
            }
            if (typeof window.Android.saveUsername === 'function') window.Android.saveUsername(username);
            if (password) {
                if (typeof window.Android.savePassword === 'function') window.Android.savePassword(password);
            } else {
                if (typeof window.Android.clearSavedPassword === 'function') window.Android.clearSavedPassword();
            }
            if (authKey) {
                if (typeof window.Android.saveAuthKey === 'function') window.Android.saveAuthKey(authKey);
            } else {
                if (typeof window.Android.clearAuthKey === 'function') window.Android.clearAuthKey();
            }
            // Reload triggers a full reconnect with the new settings
            if (typeof window.Android.reloadPage === 'function') window.Android.reloadPage();
            return;
        }

        // Save general settings
        if (setupInputHeightValue < 1) setupInputHeightValue = 1;
        if (setupInputHeightValue > 15) setupInputHeightValue = 15;
        if (setupColorOffset < 0) setupColorOffset = 0;
        if (setupColorOffset > 100) setupColorOffset = 100;

        moreModeEnabled = setupMoreMode;
        worldSwitchMode = setupWorldSwitchMode;
        ansiMusicEnabled = setupAnsiMusic;
        zwjEnabled = setupZwj;
        ttsMode = setupTtsMode.toLowerCase();
        tlsProxyEnabled = setupTlsProxy;
        newLineIndicator = setupNewLineIndicator;
        debugEnabled = setupDebug;
        guiTheme = setupGuiTheme;
        colorOffsetPercent = setupColorOffset;
        applyTheme(guiTheme);
        setInputHeight(setupInputHeightValue);
        applyTransparency(setupTransparency);
        renderOutput();

        // Save web settings (skip if multiuser)
        if (!multiuserMode) {
            webSecure = editWebSecure;
            httpEnabled = editHttpEnabled;
            httpPort = parseInt(elements.webHttpPort.value) || 9000;
            wsAllowList = elements.webAllowList.value;
            wsPassword = elements.webWsPassword ? elements.webWsPassword.value : wsPassword;
            wsCertFile = elements.webCertFile.value;
            wsKeyFile = elements.webKeyFile.value;
        }

        // Save font settings
        _saveFontSettingsInline();

        // Send combined update to server
        const msg = buildUpdateGlobalSettings();
        msg.input_height = setupInputHeightValue;
        send(msg);

        closeSettingsPopup();
    }

    function updateWebPopupUI() {
        // Update protocol select (use edit state)
        elements.webProtocolSelect.value = editWebSecure ? 'secure' : 'non-secure';

        // Update labels based on protocol
        elements.httpLabel.textContent = editWebSecure ? 'HTTPS enabled' : 'HTTP enabled';
        elements.httpPortLabel.textContent = editWebSecure ? 'HTTPS port' : 'HTTP port';
        // Update select dropdowns (use edit state)
        elements.webHttpEnabledSelect.value = editHttpEnabled ? 'on' : 'off';

        // Update input fields (from global state - text fields are read on save)
        elements.webHttpPort.value = httpPort;
        elements.webAllowList.value = wsAllowList;
        if (elements.webWsPassword) elements.webWsPassword.value = wsPassword;
        // Show placeholder if TLS configured but paths not sent from server
        if (tlsConfigured && !wsCertFile) {
            elements.webCertFile.value = '';
            elements.webCertFile.placeholder = 'Configured';
        } else {
            elements.webCertFile.value = wsCertFile;
            elements.webCertFile.placeholder = '';
        }
        if (tlsConfigured && !wsKeyFile) {
            elements.webKeyFile.value = '';
            elements.webKeyFile.placeholder = 'Configured';
        } else {
            elements.webKeyFile.value = wsKeyFile;
            elements.webKeyFile.placeholder = '';
        }

        // Show/hide TLS fields based on protocol
        elements.tlsCertField.style.display = editWebSecure ? 'flex' : 'none';
        elements.tlsKeyField.style.display = editWebSecure ? 'flex' : 'none';

        // Populate auth key field
        if (elements.webAuthKey) {
            elements.webAuthKey.value = serverAuthKey || '';
        }
    }

    // saveWebSettings removed — merged into saveSettingsAll

    // openFontPopup/closeFontPopup removed — merged into openSettingsPopup/closeSettingsPopup

    function renderFontFamilyList() {
        const list = elements.fontFamilyList;
        list.innerHTML = '';
        FONT_FAMILIES.forEach(function(entry) {
            const value = entry[0];
            const label = entry[1];
            const item = document.createElement('div');
            item.className = 'font-family-item' + (value === fontEditName ? ' selected' : '');
            item.textContent = label;
            if (value && value !== '') {
                item.style.fontFamily = "'" + value + "', monospace";
            }
            item.addEventListener('click', function() {
                fontEditName = value;
                // Update selection highlighting
                list.querySelectorAll('.font-family-item').forEach(function(el) {
                    el.classList.remove('selected');
                });
                item.classList.add('selected');
            });
            list.appendChild(item);
        });
        // Scroll selected item into view
        const selected = list.querySelector('.font-family-item.selected');
        if (selected) {
            selected.scrollIntoView({ block: 'nearest' });
        }
    }

    function updateFontPopupUI() {
        elements.fontPhoneValue.textContent = fontEditSizePhone;
        elements.fontTabletValue.textContent = fontEditSizeTablet;
        elements.fontDesktopValue.textContent = fontEditSizeDesktop;
        elements.fontWeightValue.textContent = fontEditWeight;
        if (elements.fontLineheightValue) elements.fontLineheightValue.textContent = fontEditLineHeight.toFixed(1);
        if (elements.fontLetterspacingValue) elements.fontLetterspacingValue.textContent = fontEditLetterSpacing.toFixed(1);
        if (elements.fontWordspacingValue) elements.fontWordspacingValue.textContent = fontEditWordSpacing.toFixed(1);
        // Grey out advanced section based on checkbox
        var adv = elements.fontAdvancedSection;
        var chk = elements.fontAdvancedToggle;
        if (adv && chk) {
            adv.style.opacity = chk.checked ? '1' : '0.35';
            adv.style.pointerEvents = chk.checked ? '' : 'none';
        }
    }

    // saveFontSettings removed — merged into saveSettingsAll
    function _saveFontSettingsInline() {
        // Called from saveSettingsAll — applies font changes
        applyFontFamily(fontEditName);
        webFontSizePhone = fontEditSizePhone;
        webFontSizeTablet = fontEditSizeTablet;
        webFontSizeDesktop = fontEditSizeDesktop;
        webFontWeight = fontEditWeight;
        webFontLineHeight = fontEditLineHeight;
        webFontLetterSpacing = fontEditLetterSpacing;
        webFontWordSpacing = fontEditWordSpacing;
        applyFontWeight(webFontWeight);
        applyAdvancedFontSettings();
        var fontPx = deviceType === 'phone' ? webFontSizePhone :
                     deviceType === 'tablet' ? webFontSizeTablet : webFontSizeDesktop;
        setFontSize(clampFontSize(fontPx), false);
    }

    // Worlds list popup functions (/connections, /l)
    function openWorldsPopup() {
        worldsPopupOpen = true;
        selectedWorldsRowIndex = currentWorldIndex;
        elements.worldsModal.className = 'modal visible';
        elements.worldsModal.style.display = 'flex';
        renderWorldsTable();
    }

    function closeWorldsPopup() {
        worldsPopupOpen = false;
        elements.worldsModal.className = 'modal';
        elements.worldsModal.style.display = 'none';
        elements.input.focus();
    }

    // Scroll the selected row into view in worlds table
    function scrollSelectedRowIntoView() {
        // Use requestAnimationFrame to ensure DOM is updated before scrolling
        requestAnimationFrame(() => {
            const container = document.getElementById('worlds-table-container');
            const selectedRow = container?.querySelector('tr.selected-row');
            if (selectedRow && container) {
                // Calculate if element is visible in the scrollable container
                const containerRect = container.getBoundingClientRect();
                const rowRect = selectedRow.getBoundingClientRect();

                // Check if row is above or below the visible area
                if (rowRect.top < containerRect.top) {
                    // Row is above visible area - scroll up
                    selectedRow.scrollIntoView({ block: 'start', behavior: 'auto' });
                } else if (rowRect.bottom > containerRect.bottom) {
                    // Row is below visible area - scroll down
                    selectedRow.scrollIntoView({ block: 'end', behavior: 'auto' });
                }
            }
        });
    }

    // Format elapsed seconds like the console
    function formatElapsed(secs) {
        if (secs === null || secs === undefined) return '-';
        if (secs < 60) return secs + 's';
        if (secs < 3600) return Math.floor(secs / 60) + 'm';
        if (secs < 86400) return Math.floor(secs / 3600) + 'h';
        return Math.floor(secs / 86400) + 'd';
    }

    // Format duration for /l command output
    // Under 60 minutes: Xm, 1-24 hours: X.Xh, Over 24 hours: X.Xd
    function formatDurationShort(secs) {
        if (secs === null || secs === undefined) return '—';
        const minutes = Math.floor(secs / 60);
        const hours = secs / 3600;
        const days = secs / 86400;

        if (minutes < 60) {
            return minutes + 'm';
        } else if (hours < 24) {
            return hours.toFixed(1) + 'h';
        } else {
            return days.toFixed(1) + 'd';
        }
    }

    // Format worlds list for /l command (text output)
    // Only shows connected worlds
    function formatWorldsList() {
        const KEEPALIVE_SECS = 5 * 60;
        const GRAY = '\x1b[90m';
        const YELLOW = '\x1b[33m';
        const CYAN = '\x1b[36m';
        const RESET = '\x1b[0m';

        // Filter to connected worlds only
        const connectedWorlds = worlds
            .map((world, idx) => ({ world, idx }))
            .filter(({ world }) => world.connected);

        if (connectedWorlds.length === 0) {
            return ['No worlds connected.'];
        }

        const lines = [];

        // Header line
        lines.push(padRight('World', 20) + padLeft('Unseen', 6) + '  ' +
            padLeft('Last', 9) + '  ' + padLeft('KA', 9) + '  ' +
            padLeft('Buffer', 7));

        connectedWorlds.forEach(({ world, idx }) => {
            // Current marker
            const currentMarker = idx === currentWorldIndex ?
                CYAN + '*' + RESET :
                ' ';

            // Unseen count
            const unseen = world.unseen_lines || 0;
            const unseenStr = unseen > 0 ?
                YELLOW + padLeft(unseen.toString(), 6) + RESET :
                GRAY + padLeft('—', 6) + RESET;

            // Last (recv/send combined)
            const lastSend = formatDurationShort(world.last_send_secs);
            const lastRecv = formatDurationShort(world.last_recv_secs);
            const last = lastRecv + '/' + lastSend;

            // KA (lastNOP/nextNOP combined)
            const lastNop = formatDurationShort(world.last_nop_secs);
            const lastSendVal = world.last_send_secs !== null && world.last_send_secs !== undefined ? world.last_send_secs : KEEPALIVE_SECS;
            const lastRecvVal = world.last_recv_secs !== null && world.last_recv_secs !== undefined ? world.last_recv_secs : KEEPALIVE_SECS;
            const lastActivity = Math.min(lastSendVal, lastRecvVal);
            const remaining = Math.max(0, KEEPALIVE_SECS - lastActivity);
            const nextNop = formatDurationShort(remaining);
            const ka = lastNop + '/' + nextNop;

            // Buffer size
            const bufferSize = (world.output_lines || []).length;

            // Truncate world name
            const name = world.name || '';
            const nameDisplay = name.length > 18 ? name.substring(0, 15) + '...' : name;

            lines.push(currentMarker + ' ' + padRight(nameDisplay, 18) + ' ' + unseenStr + '  ' +
                padLeft(last, 9) + '  ' + padLeft(ka, 9) + '  ' +
                padLeft(bufferSize.toString(), 7));
        });

        return lines;
    }

    // Helper: pad string to the right
    function padRight(str, len) {
        str = String(str);
        while (str.length < len) str += ' ';
        return str;
    }

    // Helper: pad string to the left
    function padLeft(str, len) {
        str = String(str);
        while (str.length < len) str = ' ' + str;
        return str;
    }

    // Add raw output lines (without %% prefix)
    function addRawOutputLines(lines, worldIndex) {
        const ts = Math.floor(Date.now() / 1000);
        if (worldIndex >= 0 && worldIndex < worlds.length) {
            lines.forEach(line => {
                const lineIndex = worlds[worldIndex].output_lines.length;
                worlds[worldIndex].output_lines.push({ text: line, ts: ts, from_server: false });
                if (worldIndex === currentWorldIndex) {
                    appendNewLine(line, ts, worldIndex, lineIndex);
                }
            });
        }
    }

    // Output worlds list as text (/l command)
    function outputWorldsList() {
        const lines = formatWorldsList();
        addRawOutputLines(lines, currentWorldIndex);
    }

    // Calculate next keepalive time
    function formatNextKA(lastSendSecs, lastRecvSecs) {
        const KEEPALIVE_SECS = 5 * 60; // 5 minutes
        const lastActivity = Math.min(
            lastSendSecs !== null && lastSendSecs !== undefined ? lastSendSecs : KEEPALIVE_SECS,
            lastRecvSecs !== null && lastRecvSecs !== undefined ? lastRecvSecs : KEEPALIVE_SECS
        );
        const remaining = Math.max(0, KEEPALIVE_SECS - lastActivity);
        if (remaining < 60) return remaining + 's';
        return Math.floor(remaining / 60) + 'm';
    }

    function renderWorldsTable() {
        elements.worldsTableBody.innerHTML = '';

        // Only show connected worlds (matching GUI behavior)
        const connectedWorlds = worlds
            .map((world, index) => ({ world, index }))
            .filter(({ world }) => world.connected);

        if (connectedWorlds.length === 0) {
            const tr = document.createElement('tr');
            const td = document.createElement('td');
            td.colSpan = 5;
            td.textContent = 'No worlds connected.';
            td.style.textAlign = 'center';
            td.style.color = '#888';
            tr.appendChild(td);
            elements.worldsTableBody.appendChild(tr);
            return;
        }

        connectedWorlds.forEach(({ world, index }, listIndex) => {
            const tr = document.createElement('tr');
            let classes = [];
            if (index === currentWorldIndex) {
                classes.push('current-world');
            }
            if (listIndex === selectedWorldsRowIndex) {
                classes.push('selected-row');
            }
            if (classes.length > 0) {
                tr.className = classes.join(' ');
            }

            // World name
            const tdName = document.createElement('td');
            tdName.textContent = stripAnsi(world.name || '(unnamed)').trim();
            tr.appendChild(tdName);

            // Unseen
            const tdUnseen = document.createElement('td');
            const unseen = world.unseen_lines || 0;
            tdUnseen.textContent = unseen > 0 ? unseen.toString() : '';
            if (unseen > 0) tdUnseen.className = 'unseen-count';
            tr.appendChild(tdUnseen);

            // Last (recv/send)
            const tdLast = document.createElement('td');
            tdLast.textContent = formatElapsed(world.last_recv_secs) + '/' + formatElapsed(world.last_send_secs);
            tr.appendChild(tdLast);

            // KA (last/next)
            const tdKA = document.createElement('td');
            tdKA.textContent = formatElapsed(world.last_nop_secs) + '/' + formatNextKA(world.last_send_secs, world.last_recv_secs);
            tr.appendChild(tdKA);

            // Buffer
            const tdBuffer = document.createElement('td');
            tdBuffer.textContent = (world.output_lines || []).length.toString();
            tr.appendChild(tdBuffer);

            // Store the actual world index for switching
            tr.dataset.worldIndex = index;

            // Click to select and double-click to switch
            tr.onclick = () => {
                selectedWorldsRowIndex = listIndex;
                renderWorldsTable();
            };
            tr.ondblclick = () => {
                switchWorldLocal(index);
                closeWorldsPopup();
            };

            elements.worldsTableBody.appendChild(tr);
        });
    }

    // World selector popup functions (/worlds)
    function openWorldSelectorPopup() {
        worldSelectorPopupOpen = true;
        selectedWorldIndex = currentWorldIndex;
        elements.worldFilter.value = '';
        elements.worldSelectorModal.className = 'modal visible';
        elements.worldSelectorModal.style.display = 'flex';
        renderWorldSelectorList();
        elements.worldFilter.focus();
    }

    function closeWorldSelectorPopup() {
        worldSelectorPopupOpen = false;
        elements.worldSelectorModal.className = 'modal';
        elements.worldSelectorModal.style.display = 'none';
        elements.input.focus();
    }

    function renderWorldSelectorList() {
        const filter = elements.worldFilter.value.toLowerCase();
        elements.worldSelectorTableBody.innerHTML = '';

        worlds.forEach((world, index) => {
            // Filter by "Only Connected" toggle
            if (worldSelectorOnlyConnected && !world.connected) {
                return;
            }

            // Filter by name, hostname, or user
            const name = (world.name || '').toLowerCase();
            const hostname = (world.settings?.hostname || '').toLowerCase();
            const user = (world.settings?.user || '').toLowerCase();

            if (filter && !name.includes(filter) && !hostname.includes(filter) && !user.includes(filter)) {
                return; // Skip non-matching worlds
            }

            const tr = document.createElement('tr');
            let classes = [];
            if (index === currentWorldIndex) {
                classes.push('current-world');
            }
            if (index === selectedWorldIndex) {
                classes.push('selected-row');
            }
            if (classes.length > 0) {
                tr.className = classes.join(' ');
            }

            // Status indicator column
            const tdStatus = document.createElement('td');
            const statusSpan = document.createElement('span');
            statusSpan.className = world.connected ? 'status-connected' : 'status-disconnected';
            statusSpan.textContent = world.connected ? '●' : '○';
            tdStatus.appendChild(statusSpan);
            tr.appendChild(tdStatus);

            // World name column
            const tdName = document.createElement('td');
            tdName.textContent = stripAnsi(world.name || '(unnamed)').trim();
            tr.appendChild(tdName);

            // Hostname column (desktop only)
            const tdHost = document.createElement('td');
            tdHost.className = 'desktop-only';
            tdHost.textContent = world.settings?.hostname || '';
            tr.appendChild(tdHost);

            // Port column (desktop only)
            const tdPort = document.createElement('td');
            tdPort.className = 'desktop-only';
            tdPort.textContent = world.settings?.port || '';
            tr.appendChild(tdPort);

            // User column (desktop only)
            const tdUser = document.createElement('td');
            tdUser.className = 'desktop-only';
            tdUser.textContent = world.settings?.user || '';
            tr.appendChild(tdUser);

            // Address column (mobile only) - combines host:port
            const tdAddress = document.createElement('td');
            tdAddress.className = 'mobile-only';
            const host = world.settings?.hostname || '';
            const port = world.settings?.port || '';
            tdAddress.textContent = host ? (port ? host + ':' + port : host) : '';
            tr.appendChild(tdAddress);

            tr.onclick = () => selectWorld(index);
            tr.ondblclick = () => {
                selectWorld(index);
                connectSelectedWorld();
            };

            elements.worldSelectorTableBody.appendChild(tr);
        });
    }

    function selectWorld(index) {
        selectedWorldIndex = index;
        renderWorldSelectorList();
        scrollSelectedWorldIntoView();
    }

    // Scroll the selected world into view in world selector table
    function scrollSelectedWorldIntoView() {
        requestAnimationFrame(() => {
            const container = document.getElementById('world-selector-table-container');
            const selectedItem = elements.worldSelectorTableBody?.querySelector('.selected-row');
            if (selectedItem && container) {
                const containerRect = container.getBoundingClientRect();
                const itemRect = selectedItem.getBoundingClientRect();

                if (itemRect.top < containerRect.top) {
                    selectedItem.scrollIntoView({ block: 'start', behavior: 'auto' });
                } else if (itemRect.bottom > containerRect.bottom) {
                    selectedItem.scrollIntoView({ block: 'end', behavior: 'auto' });
                }
            }
        });
    }

    // Get indices of worlds that match the current filter and "Only Connected" toggle
    function getFilteredWorldIndices() {
        const filter = elements.worldFilter.value.toLowerCase();
        const indices = [];
        worlds.forEach((world, index) => {
            // Filter by "Only Connected" toggle
            if (worldSelectorOnlyConnected && !world.connected) {
                return;
            }
            const name = (world.name || '').toLowerCase();
            const hostname = (world.settings?.hostname || '').toLowerCase();
            const user = (world.settings?.user || '').toLowerCase();
            if (!filter || name.includes(filter) || hostname.includes(filter) || user.includes(filter)) {
                indices.push(index);
            }
        });
        return indices;
    }

    function switchToSelectedWorld() {
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length) {
            switchWorldLocal(selectedWorldIndex);
            closeWorldSelectorPopup();
        }
    }

    function connectSelectedWorld() {
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length) {
            const world = worlds[selectedWorldIndex];
            // Switch to the world first
            switchWorldLocal(selectedWorldIndex);
            // Check if we have settings to connect
            const hostname = world.settings?.hostname || '';
            const port = world.settings?.port || '';
            const hasSettings = hostname.length > 0 && port.toString().length > 0;
            if (hasSettings) {
                // Has hostname/port - connect
                send({
                    type: 'ConnectWorld',
                    world_index: selectedWorldIndex
                });
            } else {
                // No settings - send to server to open editor
                send({
                    type: 'SendCommand',
                    world_index: currentWorldIndex,
                    command: '/worlds ' + world.name
                });
            }
            closeWorldSelectorPopup();
        }
    }

    function addNewWorld() {
        // Generate a unique world name
        let baseName = 'New World';
        let name = baseName;
        let counter = 1;
        while (worlds.some(w => w.name.toLowerCase() === name.toLowerCase())) {
            counter++;
            name = baseName + ' ' + counter;
        }
        // Send CreateWorld message - server creates the world, broadcasts WorldAdded,
        // and sends WorldCreated back to us so we can open the editor
        send({
            type: 'CreateWorld',
            name: name
        });
        closeWorldSelectorPopup();
    }

    function editSelectedWorld() {
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length) {
            openWorldEditorPopup(selectedWorldIndex);
            closeWorldSelectorPopup();
        }
    }

    // World Editor popup functions
    function openWorldEditorPopup(worldIndex) {
        // Block world editing in multiuser mode
        if (multiuserMode) {
            appendClientLine('World editing is disabled in multiuser mode.', currentWorldIndex, 'system');
            return;
        }
        if (worldIndex < 0 || worldIndex >= worlds.length) return;

        worldEditorPopupOpen = true;
        worldEditorIndex = worldIndex;
        const world = worlds[worldIndex];

        // Populate form fields
        elements.worldEditorTitle.textContent = 'World Editor';
        elements.worldEditName.value = world.name || '';
        elements.worldEditHostname.value = world.settings?.hostname || '';
        elements.worldEditPort.value = world.settings?.port || '';
        elements.worldEditUser.value = world.settings?.user || '';
        elements.worldEditPassword.value = world.settings?.password || '';
        elements.worldEditPassword.placeholder = '';
        const logEnabled = world.settings?.log_enabled || false;
        if (logEnabled) {
            elements.worldEditLoggingToggle.classList.add('active');
        } else {
            elements.worldEditLoggingToggle.classList.remove('active');
        }
        elements.worldEditKeepAliveCmd.value = world.settings?.keep_alive_cmd || '';
        if (elements.worldEditGmcpPackages) {
            elements.worldEditGmcpPackages.value = world.settings?.gmcp_packages || '';
        }
        if (elements.worldEditAutoReconnect) {
            elements.worldEditAutoReconnect.value = world.settings?.auto_reconnect_secs ?? '0';
        }

        // Set toggle and selects
        const useSsl = world.settings?.use_ssl || false;
        if (useSsl) {
            elements.worldEditSslToggle.classList.add('active');
        } else {
            elements.worldEditSslToggle.classList.remove('active');
        }

        const autoLogin = world.settings?.auto_connect_type || world.settings?.auto_login || 'Connect';
        elements.worldEditAutoLoginSelect.value = autoLogin;
        updateCustomDropdown(elements.worldEditAutoLoginSelect);

        const keepAlive = world.settings?.keep_alive_type || 'NOP';
        elements.worldEditKeepAliveSelect.value = keepAlive;
        updateKeepAliveCmdVisibility(keepAlive);
        updateCustomDropdown(elements.worldEditKeepAliveSelect);

        const encoding = world.settings?.encoding || 'UTF-8';
        elements.worldEditEncodingSelect.value = encoding;
        updateCustomDropdown(elements.worldEditEncodingSelect);

        elements.worldEditorModal.className = 'modal visible';
        elements.worldEditorModal.style.display = 'flex';
        elements.worldEditName.focus();
    }

    function closeWorldEditorPopup() {
        worldEditorPopupOpen = false;
        worldEditorIndex = -1;
        elements.worldEditorModal.className = 'modal';
        elements.worldEditorModal.style.display = 'none';
        focusInputWithKeyboard();
    }

    function updateKeepAliveCmdVisibility(keepAliveType) {
        if (keepAliveType === 'Custom') {
            elements.worldEditKeepAliveCmdField.classList.add('visible');
        } else {
            elements.worldEditKeepAliveCmdField.classList.remove('visible');
        }
    }

    function saveWorldEditor() {
        if (worldEditorIndex < 0 || worldEditorIndex >= worlds.length) return;

        // Send update to server
        send({
            type: 'UpdateWorldSettings',
            world_index: worldEditorIndex,
            name: elements.worldEditName.value,
            hostname: elements.worldEditHostname.value,
            port: elements.worldEditPort.value,
            user: elements.worldEditUser.value,
            password: elements.worldEditPassword.value,  // Empty means "not changed" (server preserves existing)
            use_ssl: elements.worldEditSslToggle.classList.contains('active'),
            log_enabled: elements.worldEditLoggingToggle.classList.contains('active'),
            encoding: elements.worldEditEncodingSelect.value,
            auto_login: elements.worldEditAutoLoginSelect.value,
            keep_alive_type: elements.worldEditKeepAliveSelect.value,
            keep_alive_cmd: elements.worldEditKeepAliveCmd.value,
            gmcp_packages: elements.worldEditGmcpPackages ? elements.worldEditGmcpPackages.value : '',
            auto_reconnect_secs: elements.worldEditAutoReconnect ? elements.worldEditAutoReconnect.value.trim() : '0'
        });

        // Update local state
        const world = worlds[worldEditorIndex];
        world.name = elements.worldEditName.value;
        if (!world.settings) world.settings = {};
        world.settings.hostname = elements.worldEditHostname.value;
        world.settings.port = elements.worldEditPort.value;
        world.settings.user = elements.worldEditUser.value;
        world.settings.password = elements.worldEditPassword.value;
        world.settings.use_ssl = elements.worldEditSslToggle.classList.contains('active');
        world.settings.log_enabled = elements.worldEditLoggingToggle.classList.contains('active');
        world.settings.encoding = elements.worldEditEncodingSelect.value;
        world.settings.auto_connect_type = elements.worldEditAutoLoginSelect.value;
        world.settings.keep_alive_type = elements.worldEditKeepAliveSelect.value;
        world.settings.keep_alive_cmd = elements.worldEditKeepAliveCmd.value;
        if (elements.worldEditGmcpPackages) {
            world.settings.gmcp_packages = elements.worldEditGmcpPackages.value;
        }
        if (elements.worldEditAutoReconnect) {
            world.settings.auto_reconnect_secs = elements.worldEditAutoReconnect.value.trim();
        }

        closeWorldEditorPopup();
    }

    function saveAndConnectWorldEditor() {
        if (worldEditorIndex < 0 || worldEditorIndex >= worlds.length) return;

        // Save the index before saveWorldEditor() resets it via closeWorldEditorPopup()
        const indexToConnect = worldEditorIndex;

        // Save first (this closes the popup and resets worldEditorIndex to -1)
        saveWorldEditor();

        // Then connect using the saved index
        send({
            type: 'ConnectWorld',
            world_index: indexToConnect
        });
    }

    function deleteWorldFromEditor() {
        if (worldEditorIndex < 0 || worldEditorIndex >= worlds.length) return;
        if (worlds.length <= 1) return;  // Can't delete last world

        const world = worlds[worldEditorIndex];
        closeWorldEditorPopup();

        // Open confirm dialog
        selectedWorldIndex = worldEditorIndex;
        worldConfirmPopupOpen = true;
        elements.worldConfirmText.textContent = `Delete world '${world.name}'?`;
        elements.worldConfirmModal.className = 'modal visible';
        elements.worldConfirmModal.style.display = 'flex';
    }

    // Open world delete confirmation popup
    function openWorldConfirmPopup() {
        if (worlds.length <= 1) {
            // Can't delete the last world
            return;
        }
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length) {
            const world = worlds[selectedWorldIndex];
            worldConfirmPopupOpen = true;
            elements.worldConfirmText.textContent = `Delete world '${world.name}'?`;
            elements.worldConfirmModal.className = 'modal visible';
            elements.worldConfirmModal.style.display = 'flex';
        }
    }

    // Close world delete confirmation popup
    function closeWorldConfirmPopup() {
        worldConfirmPopupOpen = false;
        elements.worldConfirmModal.className = 'modal';
        elements.worldConfirmModal.style.display = 'none';
    }

    // Confirm delete world
    function confirmDeleteWorld() {
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length && worlds.length > 1) {
            const world = worlds[selectedWorldIndex];
            // Send delete command to server
            send({
                type: 'DeleteWorld',
                world_index: selectedWorldIndex
            });
            closeWorldConfirmPopup();
        }
    }

    // Handle /worlds <name> command
    function handleWorldCommand(worldName) {
        // Find world by name (case-insensitive)
        const lowerName = worldName.toLowerCase();
        const worldIndex = worlds.findIndex(w =>
            (w.name || '').toLowerCase() === lowerName
        );

        if (worldIndex >= 0) {
            const world = worlds[worldIndex];
            // Switch to the world
            switchWorldLocal(worldIndex);
            // If not connected, check if we have settings to connect
            if (!world.connected) {
                const hostname = world.settings?.hostname || '';
                const port = world.settings?.port || '';
                const hasSettings = hostname.length > 0 && port.toString().length > 0;
                if (hasSettings) {
                    // Has hostname/port - connect
                    send({
                        type: 'ConnectWorld',
                        world_index: worldIndex
                    });
                } else {
                    // No settings - show error
                    appendClientLine('No connection settings configured for this world.', worldIndex);
                }
            }
        } else {
            // World not found - show error message locally
            appendClientLine(`World '${worldName}' not found.`);
        }
    }

    // Check if any popup is open
    function isAnyPopupOpen() {
        return actionsListPopupOpen || actionsEditorPopupOpen || actionsConfirmPopupOpen || worldsPopupOpen || worldSelectorPopupOpen || worldConfirmPopupOpen || settingsPopupOpen;
    }

    // Check if a world should be included in cycling (connected OR has activity)
    function isWorldActive(world) {
        return world.connected || worldHasActivity(world);
    }

    // Check if a world has activity (unseen lines only)
    // Note: pending_count is server-side more-mode concept, not meaningful for web activity
    function worldHasActivity(world) {
        return world.unseen_lines && world.unseen_lines > 0;
    }

    // Check if a world has unseen output (for pending_first prioritization)
    function worldHasPending(world) {
        return worldHasActivity(world);
    }

    // Get list of active world indices, sorted alphabetically
    function getActiveWorldIndices() {
        const activeWorlds = [];
        worlds.forEach((world, index) => {
            if (isWorldActive(world)) {
                activeWorlds.push({
                    index,
                    name: (world.name || '').toLowerCase()
                });
            }
        });
        // Sort alphabetically
        activeWorlds.sort((a, b) => a.name.localeCompare(b.name));
        return activeWorlds.map(w => w.index);
    }

    // Request next world from server (uses shared world switching logic)
    function requestNextWorld() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({
                type: 'CalculateNextWorld',
                current_index: currentWorldIndex
            }));
        }
    }

    // Request previous world from server (uses shared world switching logic)
    function requestPrevWorld() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({
                type: 'CalculatePrevWorld',
                current_index: currentWorldIndex
            }));
        }
    }

    // Request world with oldest pending/unseen output from server (Escape+w / Alt+w)
    function requestOldestPendingWorld() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({
                type: 'CalculateOldestPending',
                current_index: currentWorldIndex
            }));
        }
    }

    // Escape+key sequence tracking (mirrors console's last_escape pattern)
    let lastEscapeTime = 0;

    function isRecentEscape() {
        return (Date.now() - lastEscapeTime) < 500;
    }

    // Convert a JS KeyboardEvent to canonical key name (matching Rust format)
    // Returns null if the key should not be looked up in bindings
    function keyEventToName(e) {
        const key = e.key;
        // Handle Escape+key sequences (Esc pressed within 500ms)
        if (!e.ctrlKey && !e.altKey && !e.metaKey && isRecentEscape() && key !== 'Escape') {
            if (key === 'Backspace') return 'Esc-Backspace';
            if (key === ' ') return 'Esc-Space';
            if (key.length === 1) return 'Esc-' + key;  // preserves case: Esc-j vs Esc-J
            return null;
        }
        // Ctrl+letter
        if (e.ctrlKey && !e.altKey && !e.metaKey && key.length === 1) {
            return '^' + key.toUpperCase();
        }
        // Alt+letter (native Alt key, not escape sequence)
        if (e.altKey && !e.ctrlKey && !e.metaKey && key.length === 1) {
            return 'Esc-' + key;  // preserves case
        }
        // Alt+Backspace
        if (e.altKey && !e.ctrlKey && !e.metaKey && key === 'Backspace') {
            return 'Esc-Backspace';
        }
        // F-keys
        if (/^F(\d+)$/.test(key)) return key;
        // Special keys with modifiers
        const specialMap = {
            'ArrowUp': 'Up', 'ArrowDown': 'Down', 'ArrowLeft': 'Left', 'ArrowRight': 'Right',
            'PageUp': 'PageUp', 'PageDown': 'PageDown',
            'Home': 'Home', 'End': 'End', 'Insert': 'Insert', 'Delete': 'Delete',
            'Backspace': 'Backspace', 'Tab': 'Tab', 'Enter': 'Enter', 'Escape': 'Escape'
        };
        const mapped = specialMap[key];
        if (mapped) {
            if (e.shiftKey && !e.ctrlKey && !e.altKey) return 'Shift-' + mapped;
            if (e.ctrlKey && !e.shiftKey && !e.altKey) return 'Ctrl-' + mapped;
            if (e.altKey && !e.shiftKey && !e.ctrlKey) return 'Alt-' + mapped;
            if (!e.shiftKey && !e.ctrlKey && !e.altKey) return mapped;
        }
        return null;
    }

    // Look up a key name in keybindings and return the action ID, or null
    function lookupBinding(keyName) {
        if (!keyName) return null;
        const action = keybindings[keyName];
        if (action && action !== 'UNBOUND') return action;
        return null;
    }

    // Push text to the kill ring (for yank)
    function pushKillRing(text) {
        if (text) {
            killRing.push(text);
            if (killRing.length > 100) killRing.shift();
        }
    }

    // Yank (paste) most recent kill ring entry at cursor
    function killRingYank() {
        if (killRing.length === 0) return;
        const text = killRing[killRing.length - 1];
        const input = elements.input;
        const pos = input.selectionStart;
        const val = input.value;
        input.value = val.substring(0, pos) + text + val.substring(pos);
        input.selectionStart = input.selectionEnd = pos + text.length;
    }

    // Delete word before cursor and push to kill ring
    function deleteWordBackwardKill() {
        const input = elements.input;
        const pos = input.selectionStart;
        const text = input.value;
        let start = pos;
        while (start > 0 && text[start - 1] === ' ') start--;
        while (start > 0 && text[start - 1] !== ' ') start--;
        const killed = text.substring(start, pos);
        pushKillRing(killed);
        input.value = text.substring(0, start) + text.substring(pos);
        input.selectionStart = input.selectionEnd = start;
    }

    // Kill to end of line and push to kill ring
    function killToEndKill() {
        const input = elements.input;
        const pos = input.selectionStart;
        const killed = input.value.substring(pos);
        pushKillRing(killed);
        input.value = input.value.substring(0, pos);
        input.selectionStart = input.selectionEnd = pos;
    }

    // Clear line and push to kill ring
    function clearLineKill() {
        const input = elements.input;
        if (input.value) pushKillRing(input.value);
        input.value = '';
        historyIndex = -1;
    }

    // Delete word forward and push to kill ring
    function deleteWordForwardKill() {
        const input = elements.input;
        const pos = input.selectionStart;
        const text = input.value;
        let end = pos;
        while (end < text.length && text[end] === ' ') end++;
        while (end < text.length && text[end] !== ' ') end++;
        const killed = text.substring(pos, end);
        pushKillRing(killed);
        input.value = text.substring(0, pos) + text.substring(end);
        input.selectionStart = input.selectionEnd = pos;
    }

    // Backward kill word (punctuation-delimited) and push to kill ring
    function backwardKillWordPunctuationKill() {
        const input = elements.input;
        const pos = input.selectionStart;
        const text = input.value;
        let start = pos;
        // Skip trailing spaces
        while (start > 0 && text[start - 1] === ' ') start--;
        // Skip until space or punctuation
        const punct = /[^a-zA-Z0-9]/;
        if (start > 0 && punct.test(text[start - 1])) {
            start--;
        } else {
            while (start > 0 && !punct.test(text[start - 1]) && text[start - 1] !== ' ') start--;
        }
        const killed = text.substring(start, pos);
        pushKillRing(killed);
        input.value = text.substring(0, start) + text.substring(pos);
        input.selectionStart = input.selectionEnd = start;
    }

    // Dispatch a keybinding action by ID. Returns true if handled.
    function dispatchAction(actionId) {
        switch (actionId) {
            // Cursor
            case 'cursor_left': {
                const input = elements.input;
                if (input.selectionStart > 0) {
                    input.selectionStart = input.selectionEnd = input.selectionStart - 1;
                }
                return true;
            }
            case 'cursor_right': {
                const input = elements.input;
                if (input.selectionStart < input.value.length) {
                    input.selectionStart = input.selectionEnd = input.selectionStart + 1;
                }
                return true;
            }
            case 'cursor_word_left': {
                const input = elements.input;
                let pos = input.selectionStart;
                const text = input.value;
                while (pos > 0 && text[pos - 1] === ' ') pos--;
                while (pos > 0 && text[pos - 1] !== ' ') pos--;
                input.selectionStart = input.selectionEnd = pos;
                return true;
            }
            case 'cursor_word_right': {
                const input = elements.input;
                let pos = input.selectionStart;
                const text = input.value;
                while (pos < text.length && text[pos] !== ' ') pos++;
                while (pos < text.length && text[pos] === ' ') pos++;
                input.selectionStart = input.selectionEnd = pos;
                return true;
            }
            case 'cursor_home': {
                elements.input.selectionStart = elements.input.selectionEnd = 0;
                return true;
            }
            case 'cursor_end': {
                const len = elements.input.value.length;
                elements.input.selectionStart = elements.input.selectionEnd = len;
                return true;
            }
            case 'cursor_up':
            case 'cursor_down':
                // Multi-line cursor movement - let browser handle natively in textarea
                return false;

            // Editing
            case 'delete_backward': {
                // Let browser handle natively
                return false;
            }
            case 'delete_forward': {
                const input = elements.input;
                const pos = input.selectionStart;
                const text = input.value;
                if (pos < text.length) {
                    input.value = text.substring(0, pos) + text.substring(pos + 1);
                    input.selectionStart = input.selectionEnd = pos;
                }
                return true;
            }
            case 'delete_word_backward':
                deleteWordBackwardKill();
                return true;
            case 'delete_word_forward':
                deleteWordForwardKill();
                return true;
            case 'delete_word_backward_punct':
                backwardKillWordPunctuationKill();
                return true;
            case 'kill_to_end':
                killToEndKill();
                return true;
            case 'clear_line':
                clearLineKill();
                return true;
            case 'transpose_chars':
                transposeChars();
                return true;
            case 'literal_next':
                // Not meaningful in browser
                return true;
            case 'capitalize_word':
                transformWordForward('capitalize');
                return true;
            case 'lowercase_word':
                transformWordForward('lowercase');
                return true;
            case 'uppercase_word':
                transformWordForward('uppercase');
                return true;
            case 'collapse_spaces':
                collapseSpaces();
                return true;
            case 'goto_matching_bracket':
                gotoMatchingBracket();
                return true;
            case 'insert_last_arg':
                lastArgument();
                return true;
            case 'yank':
                killRingYank();
                return true;

            // History
            case 'history_prev':
                historyPrev();
                return true;
            case 'history_next':
                historyNext();
                return true;
            case 'history_search_backward':
                historySearchBackward();
                return true;
            case 'history_search_forward':
                historySearchForward();
                return true;

            // Scrollback
            case 'scroll_page_up': {
                const pgH = elements.outputContainer.clientHeight;
                const pgLH = (currentFontSize || 14) * 1.2;
                elements.outputContainer.scrollBy(0, -(pgH - pgLH));
                return true;
            }
            case 'scroll_page_down': {
                const pgH = elements.outputContainer.clientHeight;
                const pgLH = (currentFontSize || 14) * 1.2;
                elements.outputContainer.scrollBy(0, pgH - pgLH);
                if (isAtBottom()) {
                    const world = worlds[currentWorldIndex];
                    const serverPending = world ? (world.pending_count || 0) : 0;
                    if (pendingLines.length === 0 && serverPending === 0) {
                        paused = false;
                        linesSincePause = 0;
                        updateStatusBar();
                    } else {
                        releaseScreenful();
                    }
                }
                return true;
            }
            case 'scroll_half_page': {
                const world = worlds[currentWorldIndex];
                const serverPending = world ? (world.pending_count || 0) : 0;
                if (pendingLines.length > 0 || serverPending > 0) {
                    releaseScreenful();
                } else {
                    const halfPage = Math.floor(elements.outputContainer.clientHeight / 2);
                    elements.outputContainer.scrollBy(0, -halfPage);
                }
                return true;
            }
            case 'flush_output':
                releaseAll();
                scrollToBottom();
                return true;
            case 'selective_flush':
                selectiveFlush();
                return true;
            case 'tab_key': {
                // Try command completion first
                const inputValue = elements.input.value;
                if (inputValue.startsWith('/')) {
                    const completed = completeCommand(inputValue);
                    if (completed !== null) {
                        elements.input.value = completed;
                        const spacePos = completed.indexOf(' ');
                        const cursorPos = spacePos >= 0 ? spacePos : completed.length;
                        elements.input.setSelectionRange(cursorPos, cursorPos);
                        return true;
                    }
                }
                const world = worlds[currentWorldIndex];
                const serverPending = world ? (world.pending_count || 0) : 0;
                if (pendingLines.length > 0 || serverPending > 0) {
                    releaseScreenful();
                } else {
                    elements.outputContainer.scrollBy(0, elements.outputContainer.clientHeight);
                }
                return true;
            }

            // World
            case 'world_next':
                requestNextWorld();
                return true;
            case 'world_prev':
                requestPrevWorld();
                return true;
            case 'world_all_next':
                requestNextWorld();  // Uses same server-side logic
                return true;
            case 'world_all_prev':
                requestPrevWorld();
                return true;
            case 'world_activity':
                requestOldestPendingWorld();
                return true;
            case 'world_previous':
                requestPrevWorld();
                return true;
            case 'world_forward':
                requestNextWorld();
                return true;

            // System
            case 'help':
                if (helpPopupOpen) closeHelpPopup(); else openHelpPopup();
                return true;
            case 'redraw':
                if (worlds[currentWorldIndex]) {
                    worlds[currentWorldIndex].output_lines = worlds[currentWorldIndex].output_lines.filter(l => l.from_server !== false);
                    worlds[currentWorldIndex].output_lines.forEach(l => { l.marked_new = false; });
                    worldOutputCache[currentWorldIndex] = {};
                }
                renderOutput();
                return true;
            case 'reload':
                // Local only — never restart the remote server
                if (window.ipc && window.ipc.postMessage) {
                    window.ipc.postMessage('reload');
                } else {
                    window.location.reload();
                }
                return true;
            case 'quit':
                // No-op in web
                return true;
            case 'suspend':
                // No-op in web
                return true;
            case 'bell':
                // No-op in browser
                return true;
            case 'spell_check':
                // No-op in web (no spell checker)
                return true;

            // Clay Extensions
            case 'toggle_tags':
                showTags = !showTags;
                renderOutput();
                return true;
            case 'filter_popup':
                if (filterPopupOpen) closeFilterPopup(); else openFilterPopup();
                return true;
            case 'toggle_action_highlight':
                highlightActions = !highlightActions;
                renderOutput();
                return true;
            case 'toggle_gmcp_media':
                send({ type: 'ToggleWorldGmcp', world_index: currentWorldIndex });
                return true;
            case 'input_grow':
                if (inputHeight < 15) setInputHeight(inputHeight + 1);
                return true;
            case 'input_shrink':
                if (inputHeight > 1) setInputHeight(inputHeight - 1);
                return true;

            default:
                return false;
        }
    }

    // Toggle menu dropdown (unified - opens upward from button)
    function toggleMenu(anchorBtn) {
        menuOpen = !menuOpen;
        if (menuOpen && anchorBtn) {
            // Position dropdown above the anchor button
            const rect = anchorBtn.getBoundingClientRect();
            elements.menuDropdown.style.bottom = (window.innerHeight - rect.top + 4) + 'px';
            elements.menuDropdown.style.left = rect.left + 'px';
        }
        elements.menuDropdown.classList.toggle('visible', menuOpen);
    }

    // Close menu dropdown
    function closeMenu() {
        menuOpen = false;
        elements.menuDropdown.classList.remove('visible');
    }

    // Handle menu item click
    function handleMenuItem(action) {
        closeMenu();
        switch (action) {
            case 'help':
                openHelpPopup();
                break;
            case 'worlds':
                outputWorldsList();
                focusInputWithKeyboard();
                break;
            case 'world-selector':
                openWorldSelectorPopup();
                break;
            case 'actions':
                openActionsPopup();
                break;
            case 'setup':
                openSettingsPopup('general');
                break;
            case 'web':
                openSettingsPopup('web');
                break;
            case 'font':
                openSettingsPopup('font');
                break;
            case 'theme-editor':
                openEditorPage('theme-editor');
                break;
            case 'keybind-editor':
                openEditorPage('keybind-editor');
                break;
            case 'toggle-tags':
                showTags = !showTags;
                renderOutput();
                focusInputWithKeyboard();
                break;
            case 'filter':
                openFilterPopup();
                break;
            case 'reload':
                // Local only — never restart the remote server
                if (window.ipc && window.ipc.postMessage) {
                    window.ipc.postMessage('reload');
                } else {
                    window.location.reload();
                }
                break;
            case 'new-window':
                var nwProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                var nwHost = window.WS_HOST || window.location.hostname;
                var nwPort = (window.WS_PORT && window.WS_PORT !== 0)
                    ? window.WS_PORT : window.location.port;
                var newWindowUrl = nwProto + '://' + nwHost + ':' + nwPort + '/';
                window.open(newWindowUrl, '_blank');
                break;
            case 'resync':
                // On Android, call native reload method if available
                if (typeof Android !== 'undefined' && Android.reloadPage) {
                    Android.reloadPage();
                } else if (typeof Android !== 'undefined' && Android.hasNativeWebSocket && Android.hasNativeWebSocket()) {
                    // Fallback: close WebSocket and reconnect to get fresh state
                    if (ws) {
                        ws.close();
                        ws = null;
                    }
                    // Small delay then reconnect
                    setTimeout(function() {
                        authenticated = false;
                        hasReceivedInitialState = false;
                        connect();
                    }, 500);
                } else {
                    // Regular browser - full page reload
                    location.reload(true);
                }
                break;
            case 'clay-server':
                // Open clay server settings tab in the settings window
                openSettingsPopup('clay-server');
                break;
            case 'change-password':
                // Open password change modal (multiuser mode only)
                if (multiuserMode) {
                    showPasswordModal(true);
                }
                break;
            case 'logout':
                // Logout (multiuser mode only)
                if (multiuserMode) {
                    performLogout();
                }
                break;
        }
    }

    // Perform logout in multiuser mode
    function performLogout() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: 'Logout' }));
        }
    }

    // Set font size by pixel value (9-20)
    // If sendToServer is true (default), save the font size to the server
    function setFontSize(px, sendToServer = true) {
        px = clampFontSize(px);

        // Check if we were at the bottom before changing size
        const wasAtBottom = isAtBottom();

        currentFontSize = px;

        // Update the per-device font size variable
        if (deviceType === 'phone') {
            webFontSizePhone = px;
        } else if (deviceType === 'tablet') {
            webFontSizeTablet = px;
        } else {
            webFontSizeDesktop = px;
        }

        // Update body font size
        document.body.style.fontSize = px + 'px';

        // Sync both range sliders
        if (elements.fontSliderInput) {
            elements.fontSliderInput.value = px;
        }
        if (elements.fontSliderVal) {
            elements.fontSliderVal.textContent = px;
        }
        if (elements.navFontSlider) {
            elements.navFontSlider.value = px;
        }
        if (elements.navFontSliderVal) {
            elements.navFontSliderVal.textContent = px;
        }

        // If we were at the bottom, stay at the bottom after font size change
        if (wasAtBottom) {
            scrollToBottom();
        }

        // Re-render to update line height calculations
        updateStatusBar();

        // Save to server so it persists across sessions
        if (sendToServer && authenticated) {
            send(buildUpdateGlobalSettings());
        }

        // Update view state for synchronized more-mode (visible lines changed with font size)
        sendViewStateIfChanged();
    }

    // Setup event listeners
    function setupEventListeners() {
        // Send button
        elements.sendBtn.onclick = sendCommand;

        // Hamburger menu with long-press for device mode
        let menuLongPressTimer = null;
        let menuLongPressed = false;

        elements.menuBtn.addEventListener('mousedown', function(e) {
            menuLongPressed = false;
            menuLongPressTimer = setTimeout(function() {
                menuLongPressed = true;
                showDeviceModeModal();
            }, 2000);
        });

        elements.menuBtn.addEventListener('click', function(e) {
            if (menuLongPressTimer) {
                clearTimeout(menuLongPressTimer);
                menuLongPressTimer = null;
            }
            if (!menuLongPressed) {
                e.stopPropagation();
                toggleMenu(elements.menuBtn);
            }
            menuLongPressed = false;
        });

        elements.menuBtn.addEventListener('mouseleave', function(e) {
            if (menuLongPressTimer) {
                clearTimeout(menuLongPressTimer);
                menuLongPressTimer = null;
            }
        });

        // Touch events (for actual touch devices)
        elements.menuBtn.addEventListener('touchstart', function(e) {
            menuLongPressed = false;
            menuLongPressTimer = setTimeout(function() {
                menuLongPressed = true;
                showDeviceModeModal();
            }, 2000);
        }, { passive: true });

        elements.menuBtn.addEventListener('touchend', function(e) {
            if (menuLongPressTimer) {
                clearTimeout(menuLongPressTimer);
                menuLongPressTimer = null;
            }
            if (!menuLongPressed) {
                e.preventDefault();
                toggleMenu(elements.menuBtn);
            }
            menuLongPressed = false;
        }, { passive: false });

        // Menu items (unified dropdown)
        elements.menuDropdown.onclick = function(e) {
            e.stopPropagation();
            const item = e.target.closest('.menu-item');
            if (item) {
                handleMenuItem(item.dataset.action);
            }
        };

        // Font size range slider (status bar)
        if (elements.fontSliderInput) {
            elements.fontSliderInput.addEventListener('input', function(e) {
                e.stopPropagation();
                setFontSize(parseInt(this.value));
            });
            elements.fontSliderInput.addEventListener('click', function(e) {
                e.stopPropagation();
            });
        }

        // Font size range slider (nav bar)
        if (elements.navFontSlider) {
            elements.navFontSlider.addEventListener('input', function(e) {
                e.stopPropagation();
                setFontSize(parseInt(this.value));
            });
            elements.navFontSlider.addEventListener('click', function(e) {
                e.stopPropagation();
            });
        }

        // Nav bar menu button (with long-press for device mode)
        let navMenuLongPressTimer = null;
        let navMenuLongPressed = false;

        if (elements.navMenuBtn) {
            elements.navMenuBtn.addEventListener('mousedown', function(e) {
                navMenuLongPressed = false;
                navMenuLongPressTimer = setTimeout(function() {
                    navMenuLongPressed = true;
                    showDeviceModeModal();
                }, 2000);
            });

            elements.navMenuBtn.addEventListener('click', function(e) {
                if (navMenuLongPressTimer) {
                    clearTimeout(navMenuLongPressTimer);
                    navMenuLongPressTimer = null;
                }
                if (!navMenuLongPressed) {
                    e.stopPropagation();
                    toggleMenu(elements.navMenuBtn);
                }
                navMenuLongPressed = false;
            });

            elements.navMenuBtn.addEventListener('mouseleave', function(e) {
                if (navMenuLongPressTimer) {
                    clearTimeout(navMenuLongPressTimer);
                    navMenuLongPressTimer = null;
                }
            });

            elements.navMenuBtn.addEventListener('touchstart', function(e) {
                elements.input.focus();
                navMenuLongPressed = false;
                navMenuLongPressTimer = setTimeout(function() {
                    navMenuLongPressed = true;
                    showDeviceModeModal();
                }, 2000);
            }, { passive: true });

            elements.navMenuBtn.addEventListener('touchend', function(e) {
                if (navMenuLongPressTimer) {
                    clearTimeout(navMenuLongPressTimer);
                    navMenuLongPressTimer = null;
                }
                if (!navMenuLongPressed) {
                    e.preventDefault();
                    toggleMenu(elements.navMenuBtn);
                }
                navMenuLongPressed = false;
            }, { passive: false });
        }

        // Track button press timing for long-press detection on nav bar world arrows
        let upBtnTimer = null;
        let upBtnLongPressed = false;
        let downBtnTimer = null;
        let downBtnLongPressed = false;

        // Up button - short press: prev world, long press (1s): prev history
        function upBtnStart(e) {
            e.preventDefault();
            elements.input.focus();
            upBtnLongPressed = false;
            upBtnTimer = setTimeout(function() {
                upBtnLongPressed = true;
                if (commandHistory.length > 0) {
                    if (historyIndex === -1) {
                        historyIndex = commandHistory.length - 1;
                    } else if (historyIndex > 0) {
                        historyIndex--;
                    }
                    elements.input.value = commandHistory[historyIndex];
                }
                elements.input.focus();
            }, 1000);
        }
        function upBtnEnd(e) {
            e.preventDefault();
            e.stopPropagation();
            if (upBtnTimer) {
                clearTimeout(upBtnTimer);
                upBtnTimer = null;
            }
            if (!upBtnLongPressed) {
                requestPrevWorld();
            }
            elements.input.focus();
        }
        if (elements.navUpBtn) {
            elements.navUpBtn.addEventListener('mousedown', upBtnStart);
            elements.navUpBtn.addEventListener('mouseup', upBtnEnd);
            elements.navUpBtn.addEventListener('touchstart', upBtnStart, { passive: false });
            elements.navUpBtn.addEventListener('touchend', upBtnEnd, { passive: false });
        }

        // Down button - short press: next world, long press (1s): next history
        function downBtnStart(e) {
            e.preventDefault();
            elements.input.focus();
            downBtnLongPressed = false;
            downBtnTimer = setTimeout(function() {
                downBtnLongPressed = true;
                if (historyIndex !== -1) {
                    if (historyIndex < commandHistory.length - 1) {
                        historyIndex++;
                        elements.input.value = commandHistory[historyIndex];
                    } else {
                        historyIndex = -1;
                        elements.input.value = '';
                    }
                }
                elements.input.focus();
            }, 1000);
        }
        function downBtnEnd(e) {
            e.preventDefault();
            e.stopPropagation();
            if (downBtnTimer) {
                clearTimeout(downBtnTimer);
                downBtnTimer = null;
            }
            if (!downBtnLongPressed) {
                requestNextWorld();
            }
            elements.input.focus();
        }
        if (elements.navDownBtn) {
            elements.navDownBtn.addEventListener('mousedown', downBtnStart);
            elements.navDownBtn.addEventListener('mouseup', downBtnEnd);
            elements.navDownBtn.addEventListener('touchstart', downBtnStart, { passive: false });
            elements.navDownBtn.addEventListener('touchend', downBtnEnd, { passive: false });
        }

        // Page up/down buttons (nav bar)
        function handlePgUp() {
            const container = elements.outputContainer;
            const pageHeight = container.clientHeight * 0.9;
            container.scrollTop = Math.max(0, container.scrollTop - pageHeight);
            updateStatusBar();
        }
        function handlePgDn() {
            const container = elements.outputContainer;
            const world = worlds[currentWorldIndex];
            const serverPending = world ? (world.pending_count || 0) : 0;
            if (pendingLines.length > 0 || serverPending > 0) {
                releaseScreenful();
            } else {
                const pageHeight = container.clientHeight * 0.9;
                container.scrollTop += pageHeight;
            }
            if (isAtBottom()) {
                if (pendingLines.length === 0 && serverPending === 0) {
                    paused = false;
                    linesSincePause = 0;
                }
            }
            updateStatusBar();
        }

        if (elements.navPgUpBtn) {
            elements.navPgUpBtn.addEventListener('touchstart', function(e) {
                elements.input.focus();
            }, { passive: true });
            elements.navPgUpBtn.addEventListener('touchend', function(e) {
                e.preventDefault();
                handlePgUp();
            }, { passive: false });
            elements.navPgUpBtn.addEventListener('click', function(e) {
                handlePgUp();
            });
        }

        if (elements.navPgDnBtn) {
            elements.navPgDnBtn.addEventListener('touchstart', function(e) {
                elements.input.focus();
            }, { passive: true });
            elements.navPgDnBtn.addEventListener('touchend', function(e) {
                e.preventDefault();
                handlePgDn();
            }, { passive: false });
            elements.navPgDnBtn.addEventListener('click', function(e) {
                handlePgDn();
            });
        }

        // Click on More/History indicator to release pending lines
        elements.statusMore.addEventListener('click', function() {
            releaseScreenful();
        });

        // Click on Activity indicator to switch to world with activity
        elements.activityIndicator.addEventListener('click', function() {
            requestNextWorld();
        });

        // Track whether we're at the bottom (for resize handling)
        let wasAtBottomBeforeResize = true;

        // Update tracking on scroll
        elements.outputContainer.addEventListener('scroll', function() {
            wasAtBottomBeforeResize = isAtBottom();
        }, { passive: true });

        // Strip zero-width spaces from copied text (inserted by insertWordBreaks for wrapping)
        document.addEventListener('copy', function(e) {
            const selection = window.getSelection();
            if (selection && selection.toString().length > 0) {
                const cleaned = selection.toString().replace(/\u200B/g, '');
                e.clipboardData.setData('text/plain', cleaned);
                e.preventDefault();
            }
        });

        // Window resize handler to update separator fill and maintain scroll position
        window.addEventListener('resize', function() {
            // If we were at the bottom before resize, stay at bottom
            if (wasAtBottomBeforeResize) {
                scrollToBottom();
            }
            updateStatusBar();
            // Update view state for synchronized more-mode (visible lines may have changed)
            sendViewStateIfChanged();
        });

        // Handle mobile keyboard visibility
        if (window.visualViewport) {
            window.visualViewport.addEventListener('resize', function() {
                // If we were at bottom before keyboard appeared, stay at bottom
                if (wasAtBottomBeforeResize) {
                    scrollToBottom();
                }
                updateStatusBar();
            });
        }

        // Click anywhere to focus input and close menu
        document.body.onclick = function(e) {
            // Close menu if open
            if (menuOpen) {
                closeMenu();
            }

            // Don't steal focus if user has selected text (for copy)
            const selection = window.getSelection();
            if (selection && selection.toString().length > 0) {
                return;
            }
            // Don't steal focus from output area (allows text selection with mouse)
            if (e.target.closest('#output-container')) {
                return;
            }
            // Don't steal focus from modals, status/nav bars, or form elements
            if (!elements.authModal.classList.contains('visible') &&
                !elements.actionsListModal.classList.contains('visible') &&
                !elements.actionsEditorModal.classList.contains('visible') &&
                !elements.actionConfirmModal.classList.contains('visible') &&
                !elements.worldsModal.classList.contains('visible') &&
                !elements.worldSelectorModal.classList.contains('visible') &&
                !elements.settingsModal?.classList.contains('visible') &&
                !elements.worldEditorModal?.classList.contains('visible') &&
                !e.target.closest('#status-bar') &&
                !e.target.closest('#nav-bar') &&
                !e.target.closest('.menu-dropdown') &&
                !e.target.closest('select')) {
                elements.input.focus();
            }
        };

        // On mobile, keep keyboard visible by refocusing input when it loses focus
        if (deviceMode === 'phone' || deviceMode === 'tablet') {
            // Helper to check if any modal or menu is open
            function isAnyModalOpen() {
                return elements.authModal.classList.contains('visible') ||
                    elements.actionsListModal.classList.contains('visible') ||
                    elements.actionsEditorModal.classList.contains('visible') ||
                    elements.actionConfirmModal.classList.contains('visible') ||
                    elements.worldsModal.classList.contains('visible') ||
                    elements.worldSelectorModal.classList.contains('visible') ||
                    elements.settingsModal.classList.contains('visible') ||
                    elements.worldEditorModal?.classList.contains('visible') ||
                    elements.passwordModal?.classList.contains('visible') ||
                    filterPopupOpen ||
                    activeCustomDropdown !== null ||
                    menuOpen;
            }

            // Track mouse interaction on output area to prevent focus-stealing during text selection
            let outputPointerDown = false;
            elements.outputContainer.addEventListener('mousedown', function() {
                outputPointerDown = true;
            });
            document.addEventListener('mouseup', function() {
                // Delay clearing so blur/touchend handlers can still see the flag
                setTimeout(function() { outputPointerDown = false; }, 200);
            });

            // Global touchend handler - refocus input after any touch interaction
            document.addEventListener('touchend', function(e) {
                // Skip if mouse is interacting with output area (text selection)
                if (outputPointerDown) return;
                // Skip if touching interactive elements
                if (e.target.closest('input, textarea, button, a, select, .custom-dropdown, .menu-item, .modal')) {
                    return;
                }
                // Skip if modal is open
                if (isAnyModalOpen()) {
                    return;
                }
                // Don't steal focus if user has selected text (for copy)
                const selection = window.getSelection();
                if (selection && selection.toString().length > 0) {
                    return;
                }
                // Refocus input after a very short delay
                requestAnimationFrame(function() {
                    if (!isAnyModalOpen() && document.activeElement !== elements.input) {
                        const sel = window.getSelection();
                        if (sel && sel.toString().length > 0) return;
                        if (outputPointerDown) return;
                        focusInputWithKeyboard();
                    }
                });
            }, { passive: true });

            // Blur handler as backup
            elements.input.addEventListener('blur', function() {
                // Use requestAnimationFrame for fastest possible refocus
                requestAnimationFrame(function() {
                    // Don't refocus if mouse is interacting with output area (text selection)
                    if (outputPointerDown) return;
                    // Don't refocus if a modal is open or interacting with form elements
                    if (isAnyModalOpen() ||
                        document.activeElement?.tagName === 'SELECT' ||
                        document.activeElement?.tagName === 'INPUT' ||
                        document.activeElement?.tagName === 'TEXTAREA' ||
                        document.activeElement?.closest('.custom-dropdown')) {
                        return;
                    }
                    // Don't steal focus if user has selected text (for copy)
                    const selection = window.getSelection();
                    if (selection && selection.toString().length > 0) {
                        return;
                    }
                    // Refocus to keep keyboard visible
                    focusInputWithKeyboard();
                });
            });

            // Periodic check to ensure input stays focused (every 500ms)
            setInterval(function() {
                if (outputPointerDown) return;
                const sel = window.getSelection();
                if (sel && sel.toString().length > 0) return;
                if (!isAnyModalOpen() &&
                    document.activeElement !== elements.input &&
                    document.activeElement?.tagName !== 'SELECT' &&
                    document.activeElement?.tagName !== 'INPUT' &&
                    document.activeElement?.tagName !== 'TEXTAREA' &&
                    !document.activeElement?.closest('.custom-dropdown')) {
                    focusInputWithKeyboard();
                }
            }, 500);
        }

        // Scroll event to update status bar (for Hist indicator)
        elements.outputContainer.onscroll = function() {
            updateStatusBar();
            // If user scrolls up, trigger pause (like console behavior)
            if (moreModeEnabled && !paused && !isAtBottom()) {
                paused = true;
                updateStatusBar();
            }
            // If user scrolls to bottom, check pending state
            if (isAtBottom()) {
                const world = worlds[currentWorldIndex];
                const serverPending = world ? (world.pending_count || 0) : 0;
                if (pendingLines.length === 0 && serverPending === 0) {
                    paused = false;
                    linesSincePause = 0;
                    updateStatusBar();
                } else if (paused) {
                    // At bottom but have pending - release them
                    releaseAll();
                }
            }
        };

        // Filter input handler
        elements.filterInput.addEventListener('input', updateFilter);
        elements.filterInput.addEventListener('keydown', function(e) {
            if (e.key === 'Escape') {
                e.preventDefault();
                closeFilterPopup();
            } else if (e.key === 'F4') {
                e.preventDefault();
                closeFilterPopup();
            }
        });

        // Help popup button handlers
        if (elements.helpCloseBtn) {
            elements.helpCloseBtn.addEventListener('click', closeHelpPopup);
        }
        if (elements.helpOkBtn) {
            elements.helpOkBtn.addEventListener('click', closeHelpPopup);
        }

        // Menu popup item click handlers
        elements.menuList.querySelectorAll('.menu-item').forEach((item, i) => {
            item.addEventListener('click', () => {
                menuSelectedIndex = i;
                selectMenuItem();
            });
        });

        // Document-level keyboard handler for navigation keys
        document.onkeydown = function(e) {
            // Skip if auth modal is visible
            if (elements.authModal.classList.contains('visible')) return;

            // Prevent browser's quick find (/) and focus input instead
            // But allow '/' in any text input or textarea (e.g., web settings path fields)
            if (e.key === '/' && document.activeElement !== elements.input &&
                document.activeElement !== elements.filterInput &&
                document.activeElement !== elements.actionFilter &&
                document.activeElement !== elements.worldFilter &&
                document.activeElement.tagName !== 'INPUT' &&
                document.activeElement.tagName !== 'TEXTAREA') {
                e.preventDefault();
                elements.input.focus();
                return;
            }

            // Handle F-keys and shortcuts globally via keybinding system
            // (before popup checks which have early returns)
            {
                const keyName = keyEventToName(e);
                const action = lookupBinding(keyName);
                if (action === 'help' || action === 'toggle_tags' || action === 'filter_popup' ||
                    action === 'toggle_action_highlight' || action === 'toggle_gmcp_media') {
                    e.preventDefault();
                    e.stopPropagation();
                    dispatchAction(action);
                    return;
                }
            }

            // Handle help popup
            if (helpPopupOpen) {
                if (e.key === 'Escape' || e.key === 'Enter') {
                    e.preventDefault();
                    closeHelpPopup();
                }
                return;
            }

            // Handle menu popup
            if (menuPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeMenuPopup();
                } else if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (menuSelectedIndex > 0) {
                        menuSelectedIndex--;
                    } else {
                        menuSelectedIndex = menuItems.length - 1;
                    }
                    updateMenuSelection();
                } else if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (menuSelectedIndex < menuItems.length - 1) {
                        menuSelectedIndex++;
                    } else {
                        menuSelectedIndex = 0;
                    }
                    updateMenuSelection();
                } else if (e.key === 'Enter') {
                    e.preventDefault();
                    selectMenuItem();
                }
                return;
            }

            // Handle actions confirm popup
            if (actionsConfirmPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeActionsConfirmPopup();
                }
                return;
            }

            // Handle actions editor popup
            if (actionsEditorPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeActionsEditorPopup();
                }
                return;
            }

            // Handle actions list popup
            if (actionsListPopupOpen) {
                const filteredIndices = getFilteredActionIndices();

                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeActionsListPopup();
                } else if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (filteredIndices.length > 0) {
                        const currentPos = filteredIndices.indexOf(selectedActionIndex);
                        if (currentPos > 0) {
                            selectedActionIndex = filteredIndices[currentPos - 1];
                        } else {
                            selectedActionIndex = filteredIndices[filteredIndices.length - 1]; // Wrap to bottom
                        }
                        renderActionsList();
                    }
                } else if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (filteredIndices.length > 0) {
                        const currentPos = filteredIndices.indexOf(selectedActionIndex);
                        if (currentPos < filteredIndices.length - 1) {
                            selectedActionIndex = filteredIndices[currentPos + 1];
                        } else {
                            selectedActionIndex = filteredIndices[0]; // Wrap to top
                        }
                        renderActionsList();
                    }
                } else if (e.key === 'Enter' && document.activeElement === elements.actionFilter) {
                    // Enter in filter field opens editor for selected action
                    e.preventDefault();
                    if (selectedActionIndex >= 0 && selectedActionIndex < actions.length) {
                        openActionsEditorPopup(selectedActionIndex);
                    }
                }
                return;
            }

            // Handle worlds list popup
            if (worldsPopupOpen) {
                // Get connected worlds for navigation
                const connectedWorlds = worlds
                    .map((world, index) => ({ world, index }))
                    .filter(({ world }) => world.connected);

                if (e.key === 'Escape') {
                    e.preventDefault();
                    e.stopPropagation();
                    closeWorldsPopup();
                } else if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (connectedWorlds.length > 0) {
                        if (selectedWorldsRowIndex > 0) {
                            selectedWorldsRowIndex--;
                        } else {
                            selectedWorldsRowIndex = connectedWorlds.length - 1; // Wrap to bottom
                        }
                        renderWorldsTable();
                        scrollSelectedRowIntoView();
                    }
                } else if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (connectedWorlds.length > 0) {
                        if (selectedWorldsRowIndex < connectedWorlds.length - 1) {
                            selectedWorldsRowIndex++;
                        } else {
                            selectedWorldsRowIndex = 0; // Wrap to top
                        }
                        renderWorldsTable();
                        scrollSelectedRowIntoView();
                    }
                } else if (e.key === 'Enter') {
                    e.preventDefault();
                    e.stopPropagation();
                    if (selectedWorldsRowIndex >= 0 && selectedWorldsRowIndex < connectedWorlds.length) {
                        // Use the actual world index from connected worlds
                        const actualIndex = connectedWorlds[selectedWorldsRowIndex].index;
                        switchWorldLocal(actualIndex);
                        closeWorldsPopup();
                    }
                }
                return;
            }

            // Handle setup popup
            if (settingsPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeSettingsPopup();
                }
                return;
            }

            // Font popup keyboard handling removed — merged into settingsPopupOpen check

            // Handle world delete confirm popup
            if (worldConfirmPopupOpen) {
                if (e.key === 'Escape' || e.key === 'n' || e.key === 'N') {
                    e.preventDefault();
                    closeWorldConfirmPopup();
                } else if (e.key === 'y' || e.key === 'Y' || e.key === 'Enter') {
                    e.preventDefault();
                    confirmDeleteWorld();
                }
                return;
            }

            // Handle world selector popup
            if (worldSelectorPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeWorldSelectorPopup();
                } else if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    // Move selection up
                    const visibleWorlds = getFilteredWorldIndices();
                    const currentPos = visibleWorlds.indexOf(selectedWorldIndex);
                    if (currentPos > 0) {
                        selectWorld(visibleWorlds[currentPos - 1]);
                    } else if (visibleWorlds.length > 0) {
                        selectWorld(visibleWorlds[visibleWorlds.length - 1]);
                    }
                } else if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    // Move selection down
                    const visibleWorlds = getFilteredWorldIndices();
                    const currentPos = visibleWorlds.indexOf(selectedWorldIndex);
                    if (currentPos < visibleWorlds.length - 1) {
                        selectWorld(visibleWorlds[currentPos + 1]);
                    } else if (visibleWorlds.length > 0) {
                        selectWorld(visibleWorlds[0]);
                    }
                } else if (e.key === 'Enter') {
                    e.preventDefault();
                    connectSelectedWorld();
                }
                return;
            }

            // Handle navigation keys at document level via keybinding system
            // (skip when input is focused — the input-specific handler takes care of it)
            if (document.activeElement !== elements.input &&
                document.activeElement !== elements.filterInput) {
                const keyName = keyEventToName(e);
                const action = lookupBinding(keyName);
                if (action) {
                    // Clear escape time for Esc+key sequences that matched
                    if (isRecentEscape() && e.key !== 'Escape') lastEscapeTime = 0;
                    e.preventDefault();
                    e.stopPropagation();
                    dispatchAction(action);
                    elements.input.focus();
                    return;
                }
            }

            // Escape handling: close popups or track for sequences
            if (e.key === 'Escape' && filterPopupOpen) {
                e.preventDefault();
                closeFilterPopup();
            } else if (e.key === 'Escape' && deviceModeModalOpen) {
                e.preventDefault();
                hideDeviceModeModal();
            } else if (e.key === 'Escape') {
                lastEscapeTime = Date.now();
            }
        };

        // Keyboard controls (console-style) - input-specific
        elements.input.addEventListener('keydown', function(e) {
            // Clear history search state on non-search keys
            const keyName = keyEventToName(e);
            const action = lookupBinding(keyName);
            if (e.key !== 'Escape' && action !== 'history_search_backward' && action !== 'history_search_forward') {
                clearHistorySearch();
            }

            // Enter is always handled directly (not configurable)
            if (e.key === 'Enter') {
                e.preventDefault();
                e.stopPropagation();
                sendCommand();
                return;
            }

            // Binding-based dispatch
            if (action) {
                // Clear escape time for Esc+key sequences that matched
                if (isRecentEscape() && e.key !== 'Escape') lastEscapeTime = 0;
                const handled = dispatchAction(action);
                if (handled) {
                    e.preventDefault();
                    e.stopPropagation();
                }
                return;
            }

            // Track bare Escape for Escape+key sequences
            if (e.key === 'Escape') {
                lastEscapeTime = Date.now();
            }
        });

        // Reset command completion state when input changes (typing, not Tab)
        // Also check for temperature conversion
        elements.input.addEventListener('input', function() {
            resetCompletion();
            checkTempConversion();
        });

        // Auth submit
        elements.authSubmit.onclick = function() { authenticate(); };
        elements.authPassword.onkeydown = function(e) {
            if (e.key === 'Enter') {
                authenticate();
            }
        };
        // Auth key field Enter handler
        if (elements.authKeyInput) {
            elements.authKeyInput.onkeydown = function(e) {
                if (e.key === 'Enter') {
                    authenticate();
                }
            };
        }

        // Connection error modal buttons
        elements.connectionRetryBtn.onclick = function() {
            hideConnectionErrorModal();
            forceReconnect();
        };
        elements.connectionCancelBtn.onclick = function() {
            hideConnectionErrorModal();
            // On Android, open server settings to allow changing connection details
            if (typeof Android !== 'undefined' && Android.openServerSettings) {
                Android.openServerSettings();
            }
            // In browser, just leave it disconnected - user can refresh to try again
        };

        // Reconnect modal buttons (shown when send fails due to disconnection)
        elements.reconnectBtn.onclick = function() {
            hideReconnectModal();
            forceReconnect();
        };
        elements.reconnectCancelBtn.onclick = function() {
            hideReconnectModal();
            // Clear pending command
            pendingReconnectCommand = null;
            pendingReconnectWorldIndex = null;
        };

        // Auth username field Enter key handler (multiuser mode)
        if (elements.authUsername) {
            elements.authUsername.onkeydown = function(e) {
                if (e.key === 'Enter') {
                    elements.authPassword.focus();
                }
            };
        }

        // Password modal keyboard handlers
        if (elements.passwordOld && elements.passwordNew && elements.passwordConfirm) {
            elements.passwordOld.onkeydown = function(e) {
                if (e.key === 'Enter') elements.passwordNew.focus();
                if (e.key === 'Escape') showPasswordModal(false);
            };
            elements.passwordNew.onkeydown = function(e) {
                if (e.key === 'Enter') elements.passwordConfirm.focus();
                if (e.key === 'Escape') showPasswordModal(false);
            };
            elements.passwordConfirm.onkeydown = function(e) {
                if (e.key === 'Enter') elements.passwordSaveBtn.click();
                if (e.key === 'Escape') showPasswordModal(false);
            };
        }

        // Actions List popup
        elements.actionAddBtn.onclick = () => openActionsEditorPopup(-1);
        elements.actionEditBtn.onclick = () => {
            if (selectedActionIndex >= 0 && selectedActionIndex < actions.length) {
                openActionsEditorPopup(selectedActionIndex);
            }
        };
        elements.actionDeleteBtn.onclick = openActionsConfirmPopup;
        elements.actionCancelBtn.onclick = closeActionsListPopup;
        elements.actionsListCloseBtn.onclick = closeActionsListPopup;
        elements.actionFilter.oninput = function() {
            // Update selection if current selection is filtered out
            const visibleIndices = getFilteredActionIndices();
            if (!visibleIndices.includes(selectedActionIndex)) {
                selectedActionIndex = visibleIndices.length > 0 ? visibleIndices[0] : -1;
            }
            renderActionsList();
        };

        // Actions Editor popup
        elements.actionSaveBtn.onclick = saveAction;
        elements.actionEditorDeleteBtn.onclick = function() {
            if (editingActionIndex >= 0 && editingActionIndex < actions.length) {
                selectedActionIndex = editingActionIndex;
                openActionsConfirmPopup();
            }
        };
        elements.actionEditorCancelBtn.onclick = closeActionsEditorPopup;
        elements.actionsEditorCloseBtn.onclick = closeActionsEditorPopup;
        elements.actionMatchType.onchange = function() {
            // Update placeholder based on match type
            if (this.value === 'wildcard') {
                elements.actionPattern.placeholder = '(wildcard: * and ?, empty = manual only)';
            } else {
                elements.actionPattern.placeholder = '(regex, empty = manual only)';
            }
        };

        // actionEnabled is now a select, no onclick needed

        // Actions Confirm Delete popup
        elements.actionConfirmYesBtn.onclick = confirmDeleteAction;
        elements.actionConfirmNoBtn.onclick = closeActionsConfirmPopup;

        // Worlds list popup
        elements.worldsCloseBtn.onclick = closeWorldsPopup;
        elements.worldsListCloseBtn.onclick = closeWorldsPopup;

        // World selector popup
        elements.worldAddBtn.onclick = addNewWorld;
        elements.worldEditBtn.onclick = editSelectedWorld;
        elements.worldConnectBtn.onclick = connectSelectedWorld;
        elements.worldSelectorCancelBtn.onclick = closeWorldSelectorPopup;
        elements.worldSelectorOnlyConnected.onchange = function() {
            worldSelectorOnlyConnected = this.checked;
            // Update selection if current selection is filtered out
            if (worldSelectorOnlyConnected && selectedWorldIndex >= 0 && worlds[selectedWorldIndex] && !worlds[selectedWorldIndex].connected) {
                const connectedIdx = worlds.findIndex(w => w.connected);
                selectedWorldIndex = connectedIdx >= 0 ? connectedIdx : -1;
            }
            renderWorldSelectorList();
        };

        // World delete confirm popup
        elements.worldConfirmYesBtn.onclick = confirmDeleteWorld;
        elements.worldConfirmNoBtn.onclick = closeWorldConfirmPopup;

        // World editor popup
        elements.worldEditSaveBtn.onclick = saveWorldEditor;
        elements.worldEditCancelBtn.onclick = closeWorldEditorPopup;
        elements.worldEditConnectBtn.onclick = saveAndConnectWorldEditor;
        elements.worldEditDeleteBtn.onclick = deleteWorldFromEditor;
        elements.worldEditCloseBtn.onclick = closeWorldEditorPopup;
        elements.worldEditSslToggle.onclick = function() {
            this.classList.toggle('active');
        };
        elements.worldEditLoggingToggle.onclick = function() {
            this.classList.toggle('active');
        };
        elements.worldEditKeepAliveSelect.onchange = function() {
            updateKeepAliveCmdVisibility(this.value);
        };

        elements.worldFilter.oninput = function() {
            // Update selection if current selection is filtered out
            const visibleIndices = getFilteredWorldIndices();
            if (!visibleIndices.includes(selectedWorldIndex)) {
                selectedWorldIndex = visibleIndices.length > 0 ? visibleIndices[0] : -1;
            }
            renderWorldSelectorList();
        };

        // Setup popup
        elements.settingsCloseBtn.onclick = closeSettingsPopup;
        // Tab switching
        document.querySelectorAll('.settings-tab-btn').forEach(function(btn) {
            btn.onclick = function() {
                if (!btn.dataset.tab) return;
                if (btn.dataset.tab === 'web' && multiuserMode) return;
                switchSettingsTab(btn.dataset.tab);
            };
        });
        document.getElementById('settings-theme-editor-btn').onclick = function() {
            openEditorPage('theme-editor');
        };
        document.getElementById('settings-keybind-editor-btn').onclick = function() {
            openEditorPage('keybind-editor');
        };
        elements.setupMoreModeToggle.onclick = function() {
            setupMoreMode = !setupMoreMode;
            updateSetupPopupUI();
        };
        // Note: show tags removed from setup - controlled by F2 or /tag command
        elements.setupAnsiMusicToggle.onclick = function() {
            setupAnsiMusic = !setupAnsiMusic;
            updateSetupPopupUI();
        };
        elements.setupZwjToggle.onclick = function() {
            setupZwj = !setupZwj;
            updateSetupPopupUI();
        };
        elements.setupTtsSelect.onchange = function() {
            setupTtsMode = this.value;
        };
        if (elements.setupTtsSpeakModeSelect) {
            elements.setupTtsSpeakModeSelect.onchange = function() {
                ttsSpeakMode = this.value;
            };
        }
        elements.setupTlsProxyToggle.onclick = function() {
            setupTlsProxy = !setupTlsProxy;
            updateSetupPopupUI();
        };
        elements.setupNewLineIndicatorToggle.onclick = function() {
            setupNewLineIndicator = !setupNewLineIndicator;
            updateSetupPopupUI();
        };
        elements.setupDebugToggle.onclick = function() {
            setupDebug = !setupDebug;
            updateSetupPopupUI();
        };
        elements.setupWorldSwitchSelect.onchange = function() {
            setupWorldSwitchMode = this.value;
        };
        elements.setupHeightMinus.onclick = function() {
            if (setupInputHeightValue > 1) {
                setupInputHeightValue--;
                updateSetupPopupUI();
            }
        };
        elements.setupHeightPlus.onclick = function() {
            if (setupInputHeightValue < 15) {
                setupInputHeightValue++;
                updateSetupPopupUI();
            }
        };
        elements.setupColorOffsetMinus.onclick = function() {
            if (setupColorOffset > 0) {
                setupColorOffset = Math.max(0, setupColorOffset - 5);
                updateSetupPopupUI();
            }
        };
        elements.setupColorOffsetPlus.onclick = function() {
            if (setupColorOffset < 100) {
                setupColorOffset = Math.min(100, setupColorOffset + 5);
                updateSetupPopupUI();
            }
        };
        elements.setupThemeSelect.onchange = function() {
            setupGuiTheme = this.value.toLowerCase();
        };
        if (elements.setupTransparencySlider) {
            elements.setupTransparencySlider.oninput = function() {
                setupTransparency = parseInt(this.value, 10) / 100;
                elements.setupTransparencyValue.textContent = this.value + '%';
                // Live preview
                applyTransparency(setupTransparency);
            };
        }
        elements.settingsSaveBtn.onclick = saveSettingsAll;
        elements.settingsCancelBtn.onclick = closeSettingsPopup;

        // Web settings popup (use edit state, not global state)
        elements.webProtocolSelect.onchange = function() {
            editWebSecure = this.value === 'secure';
            updateWebPopupUI();
        };
        elements.webHttpEnabledSelect.onchange = function() {
            editHttpEnabled = this.value === 'on';
            updateWebPopupUI();
        };
        // Auth key regenerate button
        if (elements.webAuthKeyRegen) {
            elements.webAuthKeyRegen.onclick = function() {
                send({ type: 'RegenerateAuthKey' });
            };
        }
        // Web save/cancel/close handled by unified settings buttons above

        // Font popup
        // Font close/cancel/save handled by unified settings buttons
        elements.fontWeightMinus.onclick = function() {
            fontEditWeight = Math.max(1, fontEditWeight - 50);
            updateFontPopupUI();
        };
        elements.fontWeightPlus.onclick = function() {
            fontEditWeight = Math.min(900, fontEditWeight + 50);
            updateFontPopupUI();
        };
        elements.fontPhoneMinus.onclick = function() {
            fontEditSizePhone = Math.max(9, fontEditSizePhone - 1);
            updateFontPopupUI();
        };
        elements.fontPhonePlus.onclick = function() {
            fontEditSizePhone = Math.min(20, fontEditSizePhone + 1);
            updateFontPopupUI();
        };
        elements.fontTabletMinus.onclick = function() {
            fontEditSizeTablet = Math.max(9, fontEditSizeTablet - 1);
            updateFontPopupUI();
        };
        elements.fontTabletPlus.onclick = function() {
            fontEditSizeTablet = Math.min(20, fontEditSizeTablet + 1);
            updateFontPopupUI();
        };
        elements.fontDesktopMinus.onclick = function() {
            fontEditSizeDesktop = Math.max(9, fontEditSizeDesktop - 1);
            updateFontPopupUI();
        };
        elements.fontDesktopPlus.onclick = function() {
            fontEditSizeDesktop = Math.min(20, fontEditSizeDesktop + 1);
            updateFontPopupUI();
        };

        // Advanced font settings toggle
        if (elements.fontAdvancedToggle) {
            elements.fontAdvancedToggle.onchange = function() {
                updateFontPopupUI();
            };
        }
        var csAuthKeyDl = document.getElementById('cs-auth-key-download');
        if (csAuthKeyDl) {
            csAuthKeyDl.onclick = function() {
                if (!serverAuthKey || !window.Android) return;
                if (typeof window.Android.saveAuthKey === 'function') {
                    window.Android.saveAuthKey(serverAuthKey);
                    // Brief confirmation feedback on the button
                    csAuthKeyDl.textContent = '✓ Saved';
                    csAuthKeyDl.disabled = true;
                    setTimeout(function() {
                        csAuthKeyDl.textContent = 'Download';
                        csAuthKeyDl.disabled = false;
                    }, 2000);
                }
            };
        }
        if (elements.fontLineheightMinus) {
            elements.fontLineheightMinus.onclick = function() {
                fontEditLineHeight = Math.max(0.5, Math.round((fontEditLineHeight - 0.1) * 10) / 10);
                updateFontPopupUI();
            };
        }
        if (elements.fontLineheightPlus) {
            elements.fontLineheightPlus.onclick = function() {
                fontEditLineHeight = Math.min(3.0, Math.round((fontEditLineHeight + 0.1) * 10) / 10);
                updateFontPopupUI();
            };
        }
        if (elements.fontLetterspacingMinus) {
            elements.fontLetterspacingMinus.onclick = function() {
                fontEditLetterSpacing = Math.max(-5, Math.round((fontEditLetterSpacing - 0.5) * 10) / 10);
                updateFontPopupUI();
            };
        }
        if (elements.fontLetterspacingPlus) {
            elements.fontLetterspacingPlus.onclick = function() {
                fontEditLetterSpacing = Math.min(10, Math.round((fontEditLetterSpacing + 0.5) * 10) / 10);
                updateFontPopupUI();
            };
        }
        if (elements.fontWordspacingMinus) {
            elements.fontWordspacingMinus.onclick = function() {
                fontEditWordSpacing = Math.max(-5, Math.round((fontEditWordSpacing - 0.5) * 10) / 10);
                updateFontPopupUI();
            };
        }
        if (elements.fontWordspacingPlus) {
            elements.fontWordspacingPlus.onclick = function() {
                fontEditWordSpacing = Math.min(20, Math.round((fontEditWordSpacing + 0.5) * 10) / 10);
                updateFontPopupUI();
            };
        }

        // Popup help buttons
        elements.popupHelpCloseBtn.onclick = closePopupHelp;
        elements.popupHelpOkBtn.onclick = closePopupHelp;
        if (elements.settingsHelpBtn) elements.settingsHelpBtn.onclick = function() {
            var helpTab = settingsActiveTab === 'web' ? 'web' :
                          settingsActiveTab === 'font' ? 'font' :
                          settingsActiveTab === 'clay-server' ? 'clay-server' : 'setup';
            openPopupHelp(helpTab);
        };
        if (elements.worldEditHelpBtn) elements.worldEditHelpBtn.onclick = function() { openPopupHelp('worldEditor'); };
        if (elements.worldSelectorHelpBtn) elements.worldSelectorHelpBtn.onclick = function() { openPopupHelp('worldSelector'); };
        if (elements.actionsListHelpBtn) elements.actionsListHelpBtn.onclick = function() { openPopupHelp('actionsList'); };
        if (elements.actionEditorHelpBtn) elements.actionEditorHelpBtn.onclick = function() { openPopupHelp('actionEditor'); };
        if (elements.connectionsHelpBtn) elements.connectionsHelpBtn.onclick = function() { openPopupHelp('connections'); };
        if (elements.menuHelpBtn) elements.menuHelpBtn.onclick = function() { openPopupHelp('menu'); };

        // Password change modal handlers
        if (elements.passwordSaveBtn) {
            elements.passwordSaveBtn.onclick = function() {
                const oldPassword = elements.passwordOld.value;
                const newPassword = elements.passwordNew.value;
                const confirmPassword = elements.passwordConfirm.value;

                if (!oldPassword || !newPassword || !confirmPassword) {
                    elements.passwordError.textContent = 'All fields are required';
                    return;
                }
                if (newPassword !== confirmPassword) {
                    elements.passwordError.textContent = 'New passwords do not match';
                    return;
                }
                if (newPassword.length < 4) {
                    elements.passwordError.textContent = 'New password must be at least 4 characters';
                    return;
                }

                // Hash both passwords and send change request
                Promise.all([hashPassword(oldPassword), hashPassword(newPassword)]).then(([oldHash, newHash]) => {
                    send({ type: 'ChangePassword', old_password_hash: oldHash, new_password_hash: newHash });
                }).catch(err => {
                    const oldHash = sha256Fallback(oldPassword);
                    const newHash = sha256Fallback(newPassword);
                    send({ type: 'ChangePassword', old_password_hash: oldHash, new_password_hash: newHash });
                });
            };
        }
        if (elements.passwordCancelBtn) {
            elements.passwordCancelBtn.onclick = function() {
                showPasswordModal(false);
            };
        }

        // Device mode modal event handlers
        if (elements.deviceModeList) {
            elements.deviceModeList.onclick = function(e) {
                const item = e.target.closest('.menu-item');
                if (item && item.dataset.mode) {
                    applyDeviceMode(item.dataset.mode);
                }
            };
        }
        if (elements.deviceModeModal) {
            elements.deviceModeModal.onclick = function(e) {
                // Close when clicking outside the modal content
                if (e.target === elements.deviceModeModal) {
                    hideDeviceModeModal();
                }
            };
        }

        // Keepalive ping every 30 seconds
        setInterval(function() {
            if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
                send({ type: 'Ping' });
            }
        }, 30000);

        // Handle visibility change (browser sleep/wake)
        // When page becomes visible, ping the server to verify the connection is alive.
        // If pong arrives in time, resync. If not, reconnect.
        document.addEventListener('visibilitychange', function() {
            if (document.visibilityState === 'visible') {
                // Clear any previous wake check
                if (wakePongTimeout) {
                    clearTimeout(wakePongTimeout);
                    wakePongTimeout = null;
                }

                if (!ws || ws.readyState === WebSocket.CLOSED) {
                    // WebSocket is already closed, reconnect immediately
                    forceReconnect();
                } else if (ws.readyState === WebSocket.CONNECTING) {
                    // Still connecting from before sleep - kill it and start fresh
                    forceReconnect();
                } else if (ws.readyState === WebSocket.OPEN && !authenticated) {
                    // Socket open but not authenticated - stale from before sleep
                    forceReconnect();
                } else if (ws.readyState === WebSocket.OPEN && authenticated) {
                    // Socket looks open - verify with a ping
                    try {
                        ws.send(JSON.stringify({ type: 'Ping' }));
                    } catch (e) {
                        // Send failed, connection is dead
                        forceReconnect();
                        return;
                    }
                    // Wait up to 3 seconds for Pong response
                    wakePongTimeout = setTimeout(function() {
                        wakePongTimeout = null;
                        forceReconnect();
                    }, 3000);
                }
            }
        });
    }

    // Show certificate warning for wss:// self-signed cert issues
    function showCertWarning() {
        let warning = document.getElementById('cert-warning');
        if (!warning) {
            warning = document.createElement('div');
            warning.id = 'cert-warning';
            warning.style.cssText = 'position:fixed;top:10px;left:50%;transform:translateX(-50%);background:#c00;color:#fff;padding:15px 20px;border-radius:8px;z-index:2000;text-align:center;max-width:90%;';
            const host = window.location.hostname;
            const certPort = window.location.port || '443';
            const certUrl = `https://${host}:${certPort}/`;
            warning.innerHTML = `
                <div style="margin-bottom:10px;font-weight:bold;">WebSocket Connection Failed</div>
                <div style="margin-bottom:10px;">If using a self-signed certificate, you need to accept it.</div>
                <a href="${certUrl}" target="_blank" style="color:#fff;text-decoration:underline;">Click here to accept the certificate for port ${certPort}</a>
                <div style="margin-top:10px;font-size:12px;">Then refresh this page.</div>
            `;
            document.body.appendChild(warning);
        }
        warning.style.display = 'block';
    }

    function hideCertWarning() {
        const warning = document.getElementById('cert-warning');
        if (warning) {
            warning.style.display = 'none';
        }
    }

    // Expose keepalive function for Android app to call
    // This helps keep the WebSocket connection alive when screen is off
    window.keepalivePing = function() {
        if (ws && ws.readyState === WebSocket.OPEN) {
            // Send a ping to keep the connection alive
            send({ type: 'Ping' });
        }
    };

    // Expose resync function for Android app to call when messages may have been lost
    window.triggerResync = function() {
        console.log('Resync triggered by Android - requesting full state');
        if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
            // Request a full state resync from the server
            ws.send(JSON.stringify({ type: 'RequestState' }));
        }
    };

    // Heartbeat ack function for Android to verify WebView responsiveness
    window.heartbeatAck = function() { return "ok"; };

    // Called by native WebView GUI to show update status messages
    window.showUpdateStatus = function(msg) {
        appendClientLine(msg);
    };

    // Expose native WebSocket check for debugging
    window.isUsingNativeWebSocket = function() {
        return usingNativeWebSocket;
    };

    // Called by Android when the 1-hour background shutdown timer fires
    window.onBackgroundTimeout = function() {
        debugLog('Background timeout - connection closed by Android');
        authenticated = false;
        hasReceivedInitialState = false;
        // Reset all fallback state so the next reconnect (on resume) starts fresh
        connectionFailures = 0;
        triedWsFallback = false;
        usingWsFallback = false;
        useNativeWebSocket = false;
        alternateHost = null;
        triedAlternateHost = false;
        // Don't auto-reconnect here - we're in the background and Android disconnected
        // to save power. Reconnection will happen when user returns (checkConnectionOnResume).
    };

    // Called by Android onResume when interface is loaded but not connected.
    // This handles cases where the connection died in the background (timeout,
    // silent TCP death, etc.) and the visibilitychange event may not fire.
    window.checkConnectionOnResume = function() {
        debugLog('checkConnectionOnResume: ws=' + (ws ? ws.readyState : 'null') + ' auth=' + authenticated);
        if (!ws || ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) {
            // Connection is dead, reconnect
            forceReconnect();
        } else if (ws.readyState === WebSocket.CONNECTING) {
            // Stale connecting attempt, kill and retry
            forceReconnect();
        } else if (ws.readyState === WebSocket.OPEN && !authenticated) {
            // Socket open but not authenticated - stale
            forceReconnect();
        } else if (ws.readyState === WebSocket.OPEN && authenticated) {
            // Looks connected - verify with ping (same as visibilitychange handler)
            if (wakePongTimeout) {
                clearTimeout(wakePongTimeout);
                wakePongTimeout = null;
            }
            try {
                ws.send(JSON.stringify({ type: 'Ping' }));
            } catch (e) {
                forceReconnect();
                return;
            }
            wakePongTimeout = setTimeout(function() {
                wakePongTimeout = null;
                forceReconnect();
            }, 3000);
        }
    };

    // Start the app
    init();
})();
