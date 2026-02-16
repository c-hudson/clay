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
        worldEditCloseBtn: document.getElementById('world-edit-close-btn'),
        worldEditDeleteBtn: document.getElementById('world-edit-delete-btn'),
        worldEditCancelBtn: document.getElementById('world-edit-cancel-btn'),
        worldEditSaveBtn: document.getElementById('world-edit-save-btn'),
        worldEditConnectBtn: document.getElementById('world-edit-connect-btn'),
        // Web settings popup
        webModal: document.getElementById('web-modal'),
        webProtocolSelect: document.getElementById('web-protocol-select'),
        webHttpEnabledSelect: document.getElementById('web-http-enabled-select'),
        webHttpPort: document.getElementById('web-http-port'),
        webWsEnabledSelect: document.getElementById('web-ws-enabled-select'),
        webWsPort: document.getElementById('web-ws-port'),
        webAllowList: document.getElementById('web-allow-list'),
        webCertFile: document.getElementById('web-cert-file'),
        webKeyFile: document.getElementById('web-key-file'),
        tlsCertField: document.getElementById('tls-cert-field'),
        tlsKeyField: document.getElementById('tls-key-field'),
        webSaveBtn: document.getElementById('web-save-btn'),
        webCancelBtn: document.getElementById('web-cancel-btn'),
        webCloseBtn: document.getElementById('web-close-btn'),
        httpLabel: document.getElementById('http-label'),
        httpPortLabel: document.getElementById('http-port-label'),
        wsLabel: document.getElementById('ws-label'),
        wsPortLabel: document.getElementById('ws-port-label'),
        // Setup popup
        setupModal: document.getElementById('setup-modal'),
        setupCloseBtn: document.getElementById('setup-close-btn'),
        setupMoreModeToggle: document.getElementById('setup-more-mode-toggle'),
        // Note: show tags removed from setup - controlled by F2 or /tag command
        setupAnsiMusicToggle: document.getElementById('setup-ansi-music-toggle'),
        setupTlsProxyToggle: document.getElementById('setup-tls-proxy-toggle'),
        setupWorldSwitchSelect: document.getElementById('setup-world-switch-select'),
        setupInputHeightValue: document.getElementById('setup-input-height-value'),
        setupHeightMinus: document.getElementById('setup-height-minus'),
        setupHeightPlus: document.getElementById('setup-height-plus'),
        setupColorOffsetValue: document.getElementById('setup-color-offset-value'),
        setupColorOffsetMinus: document.getElementById('setup-color-offset-minus'),
        setupColorOffsetPlus: document.getElementById('setup-color-offset-plus'),
        setupThemeSelect: document.getElementById('setup-theme-select'),
        setupSaveBtn: document.getElementById('setup-save-btn'),
        setupCancelBtn: document.getElementById('setup-cancel-btn'),
        // Filter popup (F4)
        filterPopup: document.getElementById('filter-popup'),
        filterInput: document.getElementById('filter-input'),
        // Menu popup (/menu)
        menuModal: document.getElementById('menu-modal'),
        menuList: document.getElementById('menu-list')
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
    let hasReceivedInitialState = false;  // True after first InitialState (to preserve world on resync)
    let worlds = [];
    let currentWorldIndex = 0;
    let pendingReconnectCommand = null;  // Command to resend after reconnect
    let pendingReconnectWorldIndex = null;  // World index to switch to after reconnect
    let commandHistory = [];
    let historyIndex = -1;
    let connectionFailures = 0;
    let reloadReconnect = false;
    let reloadReconnectAttempts = 0;
    let inputHeight = 1;
    let splashLines = [];  // Splash screen lines for multiuser mode

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
    let webPopupOpen = false;
    let webSecure = false;
    let httpEnabled = false;
    let httpPort = 9000;
    let wsEnabled = false;
    let wsPort = 9001;
    let wsAllowList = '';
    let wsCertFile = '';
    let wsKeyFile = '';
    // Temporary editing state for web popup (only saved on Save button)
    let editWebSecure = false;
    let editHttpEnabled = false;
    let editWsEnabled = false;
    let selectedWorldIndex = -1;
    let selectedWorldsRowIndex = -1; // For worlds list popup (/connections)

    // Setup popup state
    let setupPopupOpen = false;
    let setupMoreMode = true;
    let setupWorldSwitchMode = 'Unseen First';
    // Note: show tags removed from setup - controlled by F2 or /tag command
    let setupColorOffset = 0;
    let setupAnsiMusic = true;
    let setupTlsProxy = false;
    let setupInputHeightValue = 1;
    let setupGuiTheme = 'dark';

    // Filter popup state (F4)
    let filterPopupOpen = false;
    let filterText = '';

    // Menu popup state (/menu)
    let menuPopupOpen = false;
    let menuSelectedIndex = 0;
    const menuItems = [
        { label: 'Help', command: '/help' },
        { label: 'Settings', command: '/setup' },
        { label: 'Web Settings', command: '/web' },
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
    let deviceModeOverride = null;  // null = auto, or 'phone', 'tablet', 'desktop'
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

    // MCMP (MUD Client Media Protocol) state
    let mcmpDefaultUrl = '';
    let mcmpMusicPlayer = null;    // { audio, key, name } - one music track at a time
    let mcmpSoundPlayers = {};     // key -> { audio, name }
    let mcmpMusicFadeTimer = null;

    let tlsProxyEnabled = false;  // TLS proxy for connection preservation over hot reload
    let tempConvertEnabled = false;  // Temperature conversion (32F -> 32F(0C))
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

    // ============================================================================
    // Command Definitions (single source of truth is Rust's parse_command)
    // ============================================================================

    // Internal commands for tab completion (must match Rust parse_command match arms)
    // This list is verified by test_command_parity_js_vs_rust in main.rs
    const INTERNAL_COMMANDS = [
        'help', 'version', 'quit', 'reload', 'update', 'setup', 'web', 'actions',
        'worlds', 'world', 'connections', 'l', 'connect', 'disconnect', 'dc',
        'flush', 'menu', 'send', 'keepalive', 'gag', 'ban', 'unban',
        'testmusic', 'dump', 'notify', 'addworld', 'edit', 'tag', 'tags',
        'dict', 'urban', 'translate', 'tr',
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
                // Perform word-delete if input is focused
                if (document.activeElement === elements.input) {
                    const input = elements.input;
                    const pos = input.selectionStart;
                    const text = input.value;
                    // Find start of word before cursor
                    let start = pos;
                    while (start > 0 && text[start - 1] === ' ') start--;
                    while (start > 0 && text[start - 1] !== ' ') start--;
                    input.value = text.substring(0, start) + text.substring(pos);
                    input.selectionStart = input.selectionEnd = start;
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
        connect();
        updateTime();
        setInterval(updateTime, 1000);
    }

    // Load auth key from storage (Android or localStorage)
    function loadAuthKey() {
        if (window.Android && window.Android.getAuthKey) {
            authKey = window.Android.getAuthKey();
        } else if (typeof localStorage !== 'undefined') {
            authKey = localStorage.getItem('clay_auth_key');
        }
        debugLog('loadAuthKey: ' + (authKey ? 'found key' : 'no key'));
    }

    // Save auth key to storage
    function saveAuthKey(key) {
        authKey = key;
        if (window.Android && window.Android.saveAuthKey) {
            window.Android.saveAuthKey(key);
        } else if (typeof localStorage !== 'undefined') {
            localStorage.setItem('clay_auth_key', key);
        }
        debugLog('saveAuthKey: saved key');
    }

    // Clear auth key from storage
    function clearAuthKey() {
        authKey = null;
        if (window.Android && window.Android.clearAuthKey) {
            window.Android.clearAuthKey();
        } else if (typeof localStorage !== 'undefined') {
            localStorage.removeItem('clay_auth_key');
        }
        debugLog('clearAuthKey: cleared');
    }

    // Get visible line count in output area
    function getVisibleLineCount() {
        const fontSize = currentFontSize || 14;
        const lineHeight = fontSize * 1.2; // font-size * line-height
        return Math.floor(elements.outputContainer.clientHeight / lineHeight);
    }

    // Send UpdateViewState to server for synchronized more-mode
    function sendViewStateIfChanged() {
        const visibleLines = getVisibleLineCount();
        const newState = { worldIndex: currentWorldIndex, visibleLines };
        if (!lastSentViewState ||
            lastSentViewState.worldIndex !== newState.worldIndex ||
            lastSentViewState.visibleLines !== newState.visibleLines) {
            send({
                type: 'UpdateViewState',
                world_index: currentWorldIndex,
                visible_lines: visibleLines
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
                // Acknowledge receipt to Android
                if (window.Android && window.Android.messageAck) {
                    window.Android.messageAck();
                }

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
            if (ws) ws.readyState = WebSocket.CLOSED;
            authenticated = false;
            showConnecting(false);
            connectionFailures++;

            if (window.Android && window.Android.stopBackgroundService) {
                window.Android.stopBackgroundService();
            }

            if (connectionFailures >= 2) {
                showConnectionErrorModal();
            } else {
                setTimeout(connect, 3000);
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

    function connect() {
        showConnecting(true);

        // Use WS_HOST if available (needed for Android loadDataWithBaseURL), fallback to location
        const host = window.WS_HOST || window.location.hostname;
        // Use ws:// fallback if we've already failed with wss://
        const protocol = usingWsFallback ? 'ws' : window.WS_PROTOCOL;
        const wsUrl = `${protocol}://${host}:${window.WS_PORT}`;

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
                hideCertWarning();
                showConnecting(false);

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

                // After 2 failures, show error modal instead of auto-reconnecting
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
                // Server tells us upfront if it's in multiuser mode
                if (msg.multiuser_mode) {
                    enableMultiuserAuthUI();
                }
                // Try auth key first (if not multiuser mode - keys are single-user only)
                if (!msg.multiuser_mode && authKey && tryAuthWithKey()) {
                    // Key auth attempt sent, wait for response
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
                    reloadReconnect = false;
                    reloadReconnectAttempts = 0;
                    connectionFailures = 0;
                    multiuserMode = msg.multiuser_mode || false;
                    showAuthModal(false);
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
                    // If this was a key-based auth failure, clear key and show password prompt
                    if (authKeyPending) {
                        debugLog('Key-based auth failed, clearing key and showing password prompt');
                        authKeyPending = false;
                        clearAuthKey();
                        // Show password modal - don't show error for key failure
                        showAuthModal(true);
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
                // Server sent us a new auth key after successful password auth
                if (msg.auth_key) {
                    debugLog('Received auth key from server');
                    saveAuthKey(msg.auth_key);
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
                    if (msg.settings.tls_proxy_enabled !== undefined) {
                        tlsProxyEnabled = msg.settings.tls_proxy_enabled;
                    }
                    if (msg.settings.temp_convert_enabled !== undefined) {
                        tempConvertEnabled = msg.settings.temp_convert_enabled;
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
                    if (msg.settings.ws_cert_file !== undefined) {
                        wsCertFile = msg.settings.ws_cert_file;
                    }
                    if (msg.settings.ws_key_file !== undefined) {
                        wsKeyFile = msg.settings.ws_key_file;
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
                    if (msg.settings.color_offset_percent !== undefined) {
                        colorOffsetPercent = msg.settings.color_offset_percent;
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
                }
                renderOutput();
                updateStatusBar();
                // Send initial view state for synchronized more-mode
                sendViewStateIfChanged();
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
                    if (msg.data) {
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

                        // If data doesn't end with newline, last element is a partial line
                        // (only for server data - client messages are always complete)
                        if (isFromServer && !endsWithNewline && rawLines.length > 0) {
                            partialLines[msg.world_index] = rawLines.pop();
                        }

                        rawLines.forEach(line => {
                            // Strip ANSI codes to check if line has actual content
                            // Some MUDs send trailing ANSI reset codes after newlines
                            const strippedLine = line.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '');
                            // Skip lines that are empty or whitespace-only after stripping ANSI
                            if (!strippedLine || strippedLine.trim().length === 0) {
                                return;
                            }
                            // Filter out keep-alive idler message lines
                            if (line.includes('###_idler_message_') && line.includes('_###')) {
                                return;
                            }
                            const lineIndex = world.output_lines.length;
                            const lineSeq = (msg.seq !== undefined) ? msg.seq + lineIndex : lineIndex;
                            world.output_lines.push({ text: truncateIfNeeded(line), ts: lineTs, seq: lineSeq });
                            // Verify sequence order
                            if (lineIndex > 0) {
                                const prevSeq = world.output_lines[lineIndex - 1].seq;
                                if (prevSeq !== undefined && lineSeq !== undefined && lineSeq <= prevSeq) {
                                    console.warn('SEQ MISMATCH in world ' + msg.world_index + ': idx=' + lineIndex + ' expected seq>' + prevSeq + ' got seq=' + lineSeq);
                                    send({
                                        type: 'ReportSeqMismatch',
                                        world_index: msg.world_index,
                                        expected_seq_gt: prevSeq,
                                        actual_seq: lineSeq,
                                        line_text: line.substring(0, 80),
                                        source: 'web'
                                    });
                                }
                            }
                            if (msg.world_index === currentWorldIndex) {
                                handleIncomingLine(line, lineTs, msg.world_index, lineIndex);
                            }
                            // Note: Don't track unseen_lines locally - server handles centralized tracking
                            // and sends UnseenUpdate messages to keep all clients in sync
                        });
                        if (msg.world_index !== currentWorldIndex) {
                            updateStatusBar();
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
                // Clear output buffer for this world
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].output_lines = [];
                    worlds[msg.world_index].pendingCount = 0;
                    // Clear the cache for this world
                    if (worldOutputCache[msg.world_index]) {
                        worldOutputCache[msg.world_index] = [];
                    }
                    // Clear any partial line buffer
                    partialLines[msg.world_index] = '';
                    // If it's the current world, clear the display and reset more-mode state
                    if (msg.world_index === currentWorldIndex) {
                        elements.output.innerHTML = '';
                        scrollOffset = 0;
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
                        elements.prompt.innerHTML = parseAnsi(msg.prompt);
                    } else {
                        elements.prompt.textContent = '';
                    }
                }
                break;

            case 'SetInputBuffer':
                if (msg.text != null) {
                    elements.input.value = msg.text;
                    elements.input.selectionStart = elements.input.selectionEnd = msg.text.length;
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
                    if (msg.settings.tls_proxy_enabled !== undefined) {
                        tlsProxyEnabled = msg.settings.tls_proxy_enabled;
                    }
                    if (msg.settings.temp_convert_enabled !== undefined) {
                        tempConvertEnabled = msg.settings.temp_convert_enabled;
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
                    if (msg.settings.ws_cert_file !== undefined) {
                        wsCertFile = msg.settings.ws_cert_file;
                    }
                    if (msg.settings.ws_key_file !== undefined) {
                        wsKeyFile = msg.settings.ws_key_file;
                    }
                    if (msg.settings.console_theme !== undefined) {
                        consoleTheme = msg.settings.console_theme;
                    }
                    if (msg.settings.gui_theme !== undefined) {
                        guiTheme = msg.settings.gui_theme;
                        applyTheme(guiTheme);
                    }
                    if (msg.settings.color_offset_percent !== undefined) {
                        const oldOffset = colorOffsetPercent;
                        colorOffsetPercent = msg.settings.color_offset_percent;
                        if (oldOffset !== colorOffsetPercent) {
                            renderOutput(); // Re-render with new color offset
                        }
                    }
                }
                break;

            case 'Pong':
                // Keepalive response - also used for connection health check on wake
                if (wakePongTimeout) {
                    clearTimeout(wakePongTimeout);
                    wakePongTimeout = null;
                    // Connection is alive, resync state
                    ws.send(JSON.stringify({ type: 'RequestState' }));
                }
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
                            elements.prompt.innerHTML = parseAnsi(world.prompt);
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
                            highlight_color: line.highlight_color
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
                // Response to RequestScrollback (for console clients, web clients don't use this)
                // Web clients have full history so this is typically not needed
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
    function handleIncomingLine(text, ts, worldIndex, lineIndex) {
        if (!text) return;

        const visibleLines = getVisibleLineCount();
        const threshold = Math.max(1, visibleLines - 2);

        if (paused) {
            // Already paused, queue the line info
            pendingLines.push({ text, ts, worldIndex, lineIndex });
            updateStatusBar();
        } else if (moreModeEnabled && linesSincePause >= threshold) {
            // Trigger pause
            paused = true;
            pendingLines.push({ text, ts, worldIndex, lineIndex });
            // Scroll to bottom to show what we have so far
            scrollToBottom();
            updateStatusBar();
        } else {
            // Normal display - append the line
            linesSincePause++;
            appendNewLine(text, ts, worldIndex, lineIndex);
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
            appendNewLine(item.text, item.ts, item.worldIndex, item.lineIndex);
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

    // Try to authenticate with saved auth key (passwordless)
    function tryAuthWithKey() {
        if (!authKey || !ws || ws.readyState !== WebSocket.OPEN) return false;

        debugLog('tryAuthWithKey: attempting key-based auth');
        authKeyPending = true;
        const msg = {
            type: 'AuthRequest',
            password_hash: '',  // Empty - using key instead
            auth_key: authKey
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
        if (!password) return;
        if (!ws || ws.readyState !== WebSocket.OPEN) return;

        // Store password for saving on success (Android auto-login)
        pendingAuthPassword = password;

        // Get username: prefer override (auto-login), then UI element if visible
        let username = usernameOverride || null;
        if (!username && elements.authUsername && elements.authUsernameRow.style.display !== 'none') {
            username = elements.authUsername.value.trim() || null;
        }
        // Store username for saving on success (Android auto-login)
        pendingAuthUsername = username;

        // Hash password with SHA-256
        hashPassword(password).then(hash => {
            const msg = { type: 'AuthRequest', password_hash: hash, request_key: true };
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
            const msg = { type: 'AuthRequest', password_hash: hash, request_key: true };
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
        // Don't send empty commands, or any commands if not authenticated
        if (cmd.length === 0) return;
        if (!authenticated) return;

        // Release all pending lines when sending a command
        if (paused) {
            releaseAll();
        }

        // Reset lines since pause counter on user input
        linesSincePause = 0;

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
                openWebPopup();
                break;

            case '/setup':
                openSetupPopup();
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
                // Help popup not implemented in web, just ignore
                break;

            case '/menu':
                openMenuPopup();
                break;

            case '/edit':
                // Open split-screen editor locally
                // (handled by specific client-side logic if implemented)
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
        if (index >= 0 && index < worlds.length && index !== currentWorldIndex) {
            mcmpStopAll();
            currentWorldIndex = index;
            // Reset more-mode state for new world
            paused = false;
            pendingLines = [];
            linesSincePause = 0;
            renderOutput();
            updateStatusBar();
            // Update prompt to show new world's prompt
            const world = worlds[currentWorldIndex];
            if (world && world.prompt) {
                elements.prompt.innerHTML = parseAnsi(world.prompt);
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
        elements.output.innerHTML = '';

        const world = worlds[currentWorldIndex];

        // If no world selected (multiuser mode before connecting), show splash
        if (!world) {
            if (splashLines && splashLines.length > 0) {
                renderSplashScreen();
            }
            return;
        }

        const lines = world.output_lines || [];

        // Build all lines as HTML with explicit <br> line breaks
        const htmlParts = [];
        for (let i = 0; i < lines.length; i++) {
            const lineObj = lines[i];
            if (lineObj === undefined || lineObj === null) continue;

            // Handle both old string format and new object format
            const rawLine = typeof lineObj === 'string' ? lineObj : lineObj.text;
            const lineTs = typeof lineObj === 'object' ? lineObj.ts : null;
            const lineGagged = typeof lineObj === 'object' ? lineObj.gagged : false;
            const lineHighlightColor = typeof lineObj === 'object' ? lineObj.highlight_color : null;

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

            // Format timestamp prefix if showTags is enabled
            const tsPrefix = showTags && lineTs ? `<span class="timestamp">${formatTimestamp(lineTs)}</span>` : '';

            const strippedText = showTags ? cleanLine : stripMudTag(cleanLine);
            const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
            // Skip Discord emoji conversion when showTags is enabled so users can see original text
            const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
            let html = tsPrefix + (showTags ? processed : convertDiscordEmojis(processed));

            // Apply /highlight color from action command (takes priority)
            if (lineHighlightColor !== null && lineHighlightColor !== undefined) {
                const bgColor = colorNameToCss(lineHighlightColor);
                html = `<span style="background-color: ${bgColor}; display: block;">${html}</span>`;
            }
            // Apply F8 action highlighting if enabled (and no explicit highlight color)
            else if (highlightActions && lineMatchesAction(cleanLine, world.name || '')) {
                html = `<span class="action-highlight">${html}</span>`;
            }

            htmlParts.push(html);
        }

        // Join with <br> tags for explicit line breaks
        elements.output.innerHTML = htmlParts.join('<br>');

        // Clear unseen for current world
        world.unseen_lines = 0;

        scrollToBottom();
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
            worlds[worldIndex].output_lines.push({ text: clientText, ts: ts });
            if (worldIndex === currentWorldIndex) {
                appendNewLine(clientText, ts, worldIndex, lineIndex);
            }
        }
    }

    // Append a new line to current world's output (already visible)
    function appendNewLine(text, ts, worldIndex, lineIndex) {
        // Strip newlines/carriage returns
        const cleanText = String(text).replace(/[\r\n]+/g, '');

        // Format timestamp prefix if showTags is enabled
        const tsPrefix = showTags && ts ? `<span class="timestamp">${formatTimestamp(ts)}</span>` : '';

        const strippedText = showTags ? cleanText : stripMudTag(cleanText);
        const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
        // Skip Discord emoji conversion when showTags is enabled so users can see original text
        const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
        const html = tsPrefix + (showTags ? processed : convertDiscordEmojis(processed));

        // Append to output with a <br> prefix (if not first line)
        const prefix = elements.output.childNodes.length > 0 ? '<br>' : '';
        elements.output.insertAdjacentHTML('beforeend', prefix + html);

        scheduleScrollToBottom();
    }

    // Parse ANSI escape codes (supports 16, 256, and true color)
    function parseAnsi(text) {
        // Handle various escape character representations
        // Some systems send \x1b, others might send \u001b, or the character might be escaped in JSON
        // Normalize to the standard ESC character
        text = text.replace(/\\x1b/gi, '\x1b');
        text = text.replace(/\\u001b/gi, '\x1b');
        text = text.replace(/\\e/gi, '\x1b');

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
                [0, 57, 170], [170, 34, 170], [26, 146, 170], [170, 170, 170],
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
            return [230, 237, 243]; // Default text color
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
            const baseClasses = classes.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || ['ansi-bold', 'ansi-italic', 'ansi-underline'].includes(c));

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
                } else if (code >= 30 && code <= 37) {
                    // Basic foreground colors - use bright variant if bold is active
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
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
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
                        currentFgStyle = `color:rgb(${rgb[0]},${rgb[1]},${rgb[2]});`;
                        i += 2;
                    } else if (codes[i + 1] === 2 && codes.length > i + 4) {
                        // True color mode: 38;2;R;G;B
                        const r = codes[i + 2];
                        const g = codes[i + 3];
                        const b = codes[i + 4];
                        currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
                        currentFgStyle = `color:rgb(${r},${g},${b});`;
                        i += 4;
                    }
                } else if (code === 39) {
                    // Default foreground color
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
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
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
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
        result = result.replace(/\[([0-9;]*)m/g, '');

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

    // Insert zero-width spaces after break characters in long words (>15 chars)
    // Break characters: [ ] ( ) , \ / - & = ? and spaces
    // Note: '.' is excluded because it breaks filenames (image.png) and domains awkwardly
    // Must be applied BEFORE parseAnsi (on raw text, not HTML)
    function insertWordBreaks(text) {
        const ZWSP = '\u200B'; // Zero-width space
        const BREAK_CHARS = ['[', ']', '(', ')', ',', '\\', '/', '-', '&', '=', '?', '.', ';', ' '];
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
        const urlPattern = /(\b(?:https?:\/\/|www\.)[^\s<>"']*[^\s<>"'.,;:!?\)\]}>])/gi;

        return html.replace(urlPattern, function(url) {
            // Strip zero-width spaces from href (inserted by insertWordBreaks)
            const cleanUrl = url.replace(/\u200B/g, '');
            // Add protocol if missing (for www. URLs)
            const href = cleanUrl.startsWith('www.') ? 'https://' + cleanUrl : cleanUrl;
            return `<a href="${href}" target="_blank" rel="noopener" class="output-link">${url}</a>`;
        });
    }

    // Format a timestamp for display
    // Returns "HH:MM>" for today, "DD/MM HH:MM>" for previous days
    function formatTimestamp(ts) {
        if (!ts) return '';

        // Convert seconds since epoch to Date
        const date = new Date(ts * 1000);

        const hours = date.getHours().toString().padStart(2, '0');
        const minutes = date.getMinutes().toString().padStart(2, '0');
        const day = date.getDate().toString().padStart(2, '0');
        const month = (date.getMonth() + 1).toString().padStart(2, '0');

        // Always show day/month for debugging ordering issues
        return `${day}/${month} ${hours}:${minutes}> `;
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
                    // Check if it looks like a MUD tag (contains : or parentheses)
                    if (tag.includes(':') || tag.includes('(')) {
                        // It's a MUD tag, skip it
                        let afterTag = rest.substring(endBracket + 1);
                        // Strip one space after tag if present
                        if (afterTag.startsWith(' ')) {
                            afterTag = afterTag.substring(1);
                        }
                        return leadingWs + ansiPrefix + afterTag;
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
        if (!isAtBottom()) {
            const container = elements.outputContainer;
            const fontSize = currentFontSize || 14;
            const lineHeight = fontSize * 1.2;
            const linesFromBottom = Math.floor((container.scrollHeight - container.scrollTop - container.clientHeight) / lineHeight);
            elements.moreLabel.textContent = 'History';
            elements.moreCount.textContent = formatCount(linesFromBottom);
            elements.statusMore.style.display = '';
        } else if ((paused && pendingLines.length > 0) || serverPending > 0) {
            elements.moreLabel.textContent = '\u23F8 More';
            elements.moreCount.textContent = formatCount(totalPending);
            elements.statusMore.style.display = '';
        } else {
            elements.statusMore.style.display = 'none';
        }

        // Activity badge
        if (serverActivityCount > 0) {
            elements.activityCount.textContent = serverActivityCount;
            elements.activityIndicator.style.display = '';
        } else {
            elements.activityIndicator.style.display = 'none';
        }
    }

    // Update time (24-hour format HH:MM)
    function updateTime() {
        const now = new Date();
        const hours = now.getHours().toString().padStart(2, '0');
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
        // Show Clay Server menu item only when running in Android app
        const isAndroid = typeof Android !== 'undefined' && Android.openServerSettings;
        document.querySelectorAll('.menu-clay-server').forEach(el => {
            el.style.display = isAndroid ? '' : 'none';
        });
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
        const internalCommands = ['help', 'connect', 'disconnect', 'dc', 'setup', 'world', 'worlds', 'l', 'keepalive', 'reload', 'quit', 'actions', 'gag'];
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

    // Setup popup functions (/setup)
    function openSetupPopup() {
        setupPopupOpen = true;
        // Load current values
        setupMoreMode = moreModeEnabled;
        setupWorldSwitchMode = worldSwitchMode;
        // Note: show tags removed from setup - controlled by F2 or /tag command
        setupAnsiMusic = ansiMusicEnabled;
        setupTlsProxy = tlsProxyEnabled;
        setupInputHeightValue = inputHeight;
        setupGuiTheme = guiTheme;
        setupColorOffset = colorOffsetPercent;
        elements.setupModal.className = 'modal visible';
        elements.setupModal.style.display = 'flex';
        updateSetupPopupUI();
    }

    function closeSetupPopup() {
        setupPopupOpen = false;
        elements.setupModal.className = 'modal';
        elements.setupModal.style.display = 'none';
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
        if (setupTlsProxy) {
            elements.setupTlsProxyToggle.classList.add('active');
        } else {
            elements.setupTlsProxyToggle.classList.remove('active');
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
    }

    function saveSetupSettings() {
        // Read values from UI (stepper value is already tracked)
        if (setupInputHeightValue < 1) setupInputHeightValue = 1;
        if (setupInputHeightValue > 15) setupInputHeightValue = 15;
        if (setupColorOffset < 0) setupColorOffset = 0;
        if (setupColorOffset > 100) setupColorOffset = 100;

        // Apply locally
        moreModeEnabled = setupMoreMode;
        worldSwitchMode = setupWorldSwitchMode;
        // Note: show tags removed from setup - controlled by F2 or /tag command
        ansiMusicEnabled = setupAnsiMusic;
        tlsProxyEnabled = setupTlsProxy;
        guiTheme = setupGuiTheme;
        colorOffsetPercent = setupColorOffset;
        applyTheme(guiTheme);
        setInputHeight(setupInputHeightValue);

        // Re-render output with new show_tags and color_offset settings
        renderOutput();

        // Send to server
        send({
            type: 'UpdateGlobalSettings',
            more_mode_enabled: moreModeEnabled,
            spell_check_enabled: true,
            temp_convert_enabled: tempConvertEnabled,
            world_switch_mode: worldSwitchMode,
            show_tags: showTags,
            ansi_music_enabled: ansiMusicEnabled,
            input_height: setupInputHeightValue,
            console_theme: consoleTheme,
            gui_theme: guiTheme,
            gui_transparency: 1.0,
            color_offset_percent: colorOffsetPercent,
            font_name: '',
            font_size: 14.0,
            web_font_size_phone: webFontSizePhone,
            web_font_size_tablet: webFontSizeTablet,
            web_font_size_desktop: webFontSizeDesktop,
            ws_allow_list: wsAllowList,
            web_secure: webSecure,
            http_enabled: httpEnabled,
            http_port: httpPort,
            ws_enabled: wsEnabled,
            ws_port: wsPort,
            ws_cert_file: wsCertFile,
            ws_key_file: wsKeyFile,
            tls_proxy_enabled: tlsProxyEnabled
        });

        closeSetupPopup();
    }

    // Web settings popup functions (/web)
    function openWebPopup() {
        // Block web settings in multiuser mode
        if (multiuserMode) {
            appendClientLine('Web settings are disabled in multiuser mode.', currentWorldIndex, 'system');
            return;
        }
        webPopupOpen = true;
        // Copy global state to edit state
        editWebSecure = webSecure;
        editHttpEnabled = httpEnabled;
        editWsEnabled = wsEnabled;
        elements.webModal.className = 'modal visible';
        elements.webModal.style.display = 'flex';
        updateWebPopupUI();
    }

    function closeWebPopup() {
        webPopupOpen = false;
        elements.webModal.className = 'modal';
        elements.webModal.style.display = 'none';
        focusInputWithKeyboard();
    }

    function updateWebPopupUI() {
        // Update protocol select (use edit state)
        elements.webProtocolSelect.value = editWebSecure ? 'secure' : 'non-secure';

        // Update labels based on protocol
        elements.httpLabel.textContent = editWebSecure ? 'HTTPS enabled' : 'HTTP enabled';
        elements.httpPortLabel.textContent = editWebSecure ? 'HTTPS port' : 'HTTP port';
        elements.wsLabel.textContent = editWebSecure ? 'WSS enabled' : 'WS enabled';
        elements.wsPortLabel.textContent = editWebSecure ? 'WSS port' : 'WS port';

        // Update select dropdowns (use edit state)
        elements.webHttpEnabledSelect.value = editHttpEnabled ? 'on' : 'off';
        elements.webWsEnabledSelect.value = editWsEnabled ? 'on' : 'off';

        // Update input fields (from global state - text fields are read on save)
        elements.webHttpPort.value = httpPort;
        elements.webWsPort.value = wsPort;
        elements.webAllowList.value = wsAllowList;
        elements.webCertFile.value = wsCertFile;
        elements.webKeyFile.value = wsKeyFile;

        // Show/hide TLS fields based on protocol
        elements.tlsCertField.style.display = editWebSecure ? 'flex' : 'none';
        elements.tlsKeyField.style.display = editWebSecure ? 'flex' : 'none';
    }

    function saveWebSettings() {
        // Copy edit state to global state
        webSecure = editWebSecure;
        httpEnabled = editHttpEnabled;
        wsEnabled = editWsEnabled;

        // Read text field values from UI
        httpPort = parseInt(elements.webHttpPort.value) || 9000;
        wsPort = parseInt(elements.webWsPort.value) || 9001;
        wsAllowList = elements.webAllowList.value;
        wsCertFile = elements.webCertFile.value;
        wsKeyFile = elements.webKeyFile.value;

        // Send to server
        send({
            type: 'UpdateGlobalSettings',
            more_mode_enabled: moreModeEnabled,
            spell_check_enabled: true,
            temp_convert_enabled: tempConvertEnabled,
            world_switch_mode: worldSwitchMode,
            show_tags: showTags,
            ansi_music_enabled: ansiMusicEnabled,
            input_height: inputHeight,
            console_theme: consoleTheme,
            gui_theme: guiTheme,
            gui_transparency: 1.0,
            font_name: '',
            font_size: 14.0,
            web_font_size_phone: webFontSizePhone,
            web_font_size_tablet: webFontSizeTablet,
            web_font_size_desktop: webFontSizeDesktop,
            ws_allow_list: wsAllowList,
            web_secure: webSecure,
            http_enabled: httpEnabled,
            http_port: httpPort,
            ws_enabled: wsEnabled,
            ws_port: wsPort,
            ws_cert_file: wsCertFile,
            ws_key_file: wsKeyFile,
            tls_proxy_enabled: tlsProxyEnabled
        });

        closeWebPopup();
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
                worlds[worldIndex].output_lines.push({ text: line, ts: ts });
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
        // Send command to create and edit new world
        send({
            type: 'SendCommand',
            command: '/worlds ' + name,
            world_index: currentWorldIndex
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

        // Set toggle and selects
        const useSsl = world.settings?.use_ssl || false;
        if (useSsl) {
            elements.worldEditSslToggle.classList.add('active');
        } else {
            elements.worldEditSslToggle.classList.remove('active');
        }

        const autoLogin = world.settings?.auto_login || 'Connect';
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
            password: elements.worldEditPassword.value,
            use_ssl: elements.worldEditSslToggle.classList.contains('active'),
            log_enabled: elements.worldEditLoggingToggle.classList.contains('active'),
            encoding: elements.worldEditEncodingSelect.value,
            auto_login: elements.worldEditAutoLoginSelect.value,
            keep_alive_type: elements.worldEditKeepAliveSelect.value,
            keep_alive_cmd: elements.worldEditKeepAliveCmd.value,
            gmcp_packages: elements.worldEditGmcpPackages ? elements.worldEditGmcpPackages.value : ''
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
        world.settings.auto_login = elements.worldEditAutoLoginSelect.value;
        world.settings.keep_alive_type = elements.worldEditKeepAliveSelect.value;
        world.settings.keep_alive_cmd = elements.worldEditKeepAliveCmd.value;
        if (elements.worldEditGmcpPackages) {
            world.settings.gmcp_packages = elements.worldEditGmcpPackages.value;
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
        return actionsListPopupOpen || actionsEditorPopupOpen || actionsConfirmPopupOpen || worldsPopupOpen || worldSelectorPopupOpen || worldConfirmPopupOpen || webPopupOpen || setupPopupOpen;
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
                openSetupPopup();
                break;
            case 'web':
                openWebPopup();
                break;
            case 'toggle-tags':
                showTags = !showTags;
                renderOutput();
                focusInputWithKeyboard();
                break;
            case 'filter':
                openFilterPopup();
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
                // Disconnect from WebSocket and go to server settings (Android app)
                if (ws) {
                    ws.close();
                    ws = null;
                }
                // Call Android interface if available (running in Android WebView)
                if (typeof Android !== 'undefined' && Android.openServerSettings) {
                    Android.openServerSettings();
                } else {
                    // Fallback for browser: show a message
                    appendClientLine('Clay Server settings only available in Android app.');
                }
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
            send({
                type: 'UpdateGlobalSettings',
                more_mode_enabled: moreModeEnabled,
                spell_check_enabled: true,
                world_switch_mode: worldSwitchMode,
                show_tags: showTags,
                ansi_music_enabled: ansiMusicEnabled,
                input_height: inputHeight,
                console_theme: consoleTheme,
                gui_theme: guiTheme,
                gui_transparency: 1.0,
                font_name: '',
                font_size: 14.0,
                web_font_size_phone: webFontSizePhone,
                web_font_size_tablet: webFontSizeTablet,
                web_font_size_desktop: webFontSizeDesktop,
                ws_allow_list: wsAllowList,
                web_secure: webSecure,
                http_enabled: httpEnabled,
                http_port: httpPort,
                ws_enabled: wsEnabled,
                ws_port: wsPort,
                ws_cert_file: wsCertFile,
                ws_key_file: wsKeyFile,
                tls_proxy_enabled: tlsProxyEnabled
            });
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

        // Track whether we're at the bottom (for resize handling)
        let wasAtBottomBeforeResize = true;

        // Update tracking on scroll
        elements.outputContainer.addEventListener('scroll', function() {
            wasAtBottomBeforeResize = isAtBottom();
        }, { passive: true });

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
            // Don't steal focus from modals, status/nav bars, or form elements
            if (!elements.authModal.classList.contains('visible') &&
                !elements.actionsListModal.classList.contains('visible') &&
                !elements.actionsEditorModal.classList.contains('visible') &&
                !elements.actionConfirmModal.classList.contains('visible') &&
                !elements.worldsModal.classList.contains('visible') &&
                !elements.worldSelectorModal.classList.contains('visible') &&
                !elements.setupModal?.classList.contains('visible') &&
                !elements.worldEditorModal?.classList.contains('visible') &&
                !elements.webModal?.classList.contains('visible') &&
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
                    elements.webModal.classList.contains('visible') ||
                    elements.setupModal.classList.contains('visible') ||
                    elements.worldEditorModal?.classList.contains('visible') ||
                    elements.passwordModal?.classList.contains('visible') ||
                    filterPopupOpen ||
                    activeCustomDropdown !== null ||
                    menuOpen;
            }

            // Global touchend handler - refocus input after any touch interaction
            document.addEventListener('touchend', function(e) {
                // Skip if touching interactive elements
                if (e.target.closest('input, textarea, button, a, select, .custom-dropdown, .menu-item, .modal')) {
                    return;
                }
                // Skip if modal is open
                if (isAnyModalOpen()) {
                    return;
                }
                // Refocus input after a very short delay
                requestAnimationFrame(function() {
                    if (!isAnyModalOpen() && document.activeElement !== elements.input) {
                        focusInputWithKeyboard();
                    }
                });
            }, { passive: true });

            // Blur handler as backup
            elements.input.addEventListener('blur', function() {
                // Use requestAnimationFrame for fastest possible refocus
                requestAnimationFrame(function() {
                    // Don't refocus if a modal is open or interacting with form elements
                    if (isAnyModalOpen() ||
                        document.activeElement?.tagName === 'SELECT' ||
                        document.activeElement?.tagName === 'INPUT' ||
                        document.activeElement?.tagName === 'TEXTAREA' ||
                        document.activeElement?.closest('.custom-dropdown')) {
                        return;
                    }
                    // Refocus to keep keyboard visible
                    focusInputWithKeyboard();
                });
            });

            // Periodic check to ensure input stays focused (every 500ms)
            setInterval(function() {
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
            if (e.key === '/' && document.activeElement !== elements.input &&
                document.activeElement !== elements.filterInput &&
                document.activeElement !== elements.actionFilter &&
                document.activeElement !== elements.worldFilter) {
                e.preventDefault();
                elements.input.focus();
                return;
            }

            // Handle F-keys and shortcuts globally (before popup checks which have early returns)
            if (e.key === 'F2') {
                // F2: Toggle MUD tag display
                e.preventDefault();
                showTags = !showTags;
                renderOutput();
                return;
            } else if (e.key === 'F8') {
                // F8: Toggle action highlighting
                e.preventDefault();
                e.stopPropagation();
                highlightActions = !highlightActions;
                renderOutput();
                return;
            } else if (e.key === 'F9') {
                // F9: Toggle GMCP user processing for current world
                e.preventDefault();
                send({ type: 'ToggleWorldGmcp', world_index: currentWorldIndex });
                return;
            } else if (e.key === 'F4') {
                // F4: Toggle filter popup
                e.preventDefault();
                if (filterPopupOpen) {
                    closeFilterPopup();
                } else {
                    openFilterPopup();
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
            if (setupPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeSetupPopup();
                }
                return;
            }

            // Handle web settings popup
            if (webPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeWebPopup();
                }
                return;
            }

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
                    switchToSelectedWorld();
                }
                return;
            }

            // Handle navigation keys at document level
            if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey) {
                e.preventDefault();
                const world = worlds[currentWorldIndex];
                const serverPending = world ? (world.pending_count || 0) : 0;
                if (pendingLines.length > 0 || serverPending > 0) {
                    // Release one screenful of pending lines
                    releaseScreenful();
                } else {
                    // Scroll down one screenful (like more)
                    elements.outputContainer.scrollBy(0, elements.outputContainer.clientHeight);
                }
                elements.input.focus();
            } else if (e.key === 'j' && e.altKey) {
                e.preventDefault();
                releaseAll();
                scrollToBottom();
                elements.input.focus();
            } else if (e.key === 'PageUp') {
                e.preventDefault();
                elements.outputContainer.scrollBy(0, -elements.outputContainer.clientHeight);
            } else if (e.key === 'PageDown') {
                e.preventDefault();
                elements.outputContainer.scrollBy(0, elements.outputContainer.clientHeight);
                if (isAtBottom()) {
                    const world = worlds[currentWorldIndex];
                    const serverPending = world ? (world.pending_count || 0) : 0;
                    if (pendingLines.length === 0 && serverPending === 0) {
                        paused = false;
                        linesSincePause = 0;
                        updateStatusBar();
                    } else {
                        // Release one screenful when at bottom with pending (same as Tab)
                        releaseScreenful();
                    }
                }
            } else if (e.key === 'ArrowUp' && !e.ctrlKey && !e.shiftKey && !e.altKey && document.activeElement !== elements.input) {
                // Up: Switch to previous active world (request from server)
                e.preventDefault();
                requestPrevWorld();
                elements.input.focus();
            } else if (e.key === 'ArrowDown' && !e.ctrlKey && !e.shiftKey && !e.altKey && document.activeElement !== elements.input) {
                // Down: Switch to next active world (request from server)
                e.preventDefault();
                requestNextWorld();
                elements.input.focus();
            } else if (e.key === 'Escape' && filterPopupOpen) {
                // Escape: Close filter popup if open
                e.preventDefault();
                closeFilterPopup();
            } else if (e.key === 'Escape' && deviceModeModalOpen) {
                // Escape: Close device mode modal if open
                e.preventDefault();
                hideDeviceModeModal();
            }
        };

        // Keyboard controls (console-style) - input-specific
        elements.input.addEventListener('keydown', function(e) {
            if (e.key === 'Enter') {
                // Send command (also releases all pending) - both Enter and Shift+Enter
                e.preventDefault();
                e.stopPropagation();  // Prevent document-level handler from catching this
                sendCommand();
            } else if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey) {
                e.preventDefault(); // Always prevent default tab behavior
                // Try command completion first if input starts with /
                const inputValue = elements.input.value;
                if (inputValue.startsWith('/')) {
                    const completed = completeCommand(inputValue);
                    if (completed !== null) {
                        elements.input.value = completed;
                        // Move cursor to end of command part
                        const spacePos = completed.indexOf(' ');
                        const cursorPos = spacePos >= 0 ? spacePos : completed.length;
                        elements.input.setSelectionRange(cursorPos, cursorPos);
                        return;
                    }
                }
                // Tab: Release one screenful of pending lines, or scroll down
                const tabWorld = worlds[currentWorldIndex];
                const tabServerPending = tabWorld ? (tabWorld.pending_count || 0) : 0;
                if (pendingLines.length > 0 || tabServerPending > 0) {
                    releaseScreenful();
                } else {
                    // Scroll down one screenful (like more)
                    elements.outputContainer.scrollBy(0, elements.outputContainer.clientHeight);
                }
            } else if (e.key === 'j' && e.altKey) {
                // Alt+j: Jump to end, release all pending
                e.preventDefault();
                releaseAll();
                scrollToBottom();
            } else if (e.key === 'ArrowUp' && e.altKey) {
                // Alt+Up: Increase input height
                e.preventDefault();
                if (inputHeight < 15) {
                    setInputHeight(inputHeight + 1);
                }
            } else if (e.key === 'ArrowDown' && e.altKey) {
                // Alt+Down: Decrease input height
                e.preventDefault();
                if (inputHeight > 1) {
                    setInputHeight(inputHeight - 1);
                }
            } else if (e.key === 'ArrowUp' && !e.ctrlKey && !e.shiftKey && !e.altKey) {
                // Up: Switch to previous active world (request from server)
                // Ctrl+Up lets browser handle cursor movement in multi-line input
                e.preventDefault();
                requestPrevWorld();
            } else if (e.key === 'ArrowDown' && !e.ctrlKey && !e.shiftKey && !e.altKey) {
                // Down: Switch to next active world (request from server)
                // Ctrl+Down lets browser handle cursor movement in multi-line input
                e.preventDefault();
                requestNextWorld();
            } else if (e.key === 'p' && e.ctrlKey) {
                // Ctrl+P: Previous command in history
                e.preventDefault();
                if (commandHistory.length > 0) {
                    if (historyIndex === -1) {
                        historyIndex = commandHistory.length - 1;
                    } else if (historyIndex > 0) {
                        historyIndex--;
                    }
                    elements.input.value = commandHistory[historyIndex];
                }
            } else if (e.key === 'n' && e.ctrlKey) {
                // Ctrl+N: Next command in history
                e.preventDefault();
                if (historyIndex !== -1) {
                    if (historyIndex < commandHistory.length - 1) {
                        historyIndex++;
                        elements.input.value = commandHistory[historyIndex];
                    } else {
                        historyIndex = -1;
                        elements.input.value = '';
                    }
                }
            } else if (e.key === 'u' && e.ctrlKey) {
                // Ctrl+U: Clear input
                e.preventDefault();
                elements.input.value = '';
                historyIndex = -1;
            } else if (e.key === 'a' && e.ctrlKey) {
                // Ctrl+A: Move cursor to beginning of line
                e.preventDefault();
                elements.input.selectionStart = elements.input.selectionEnd = 0;
            // Note: Ctrl+W handled by window capture-phase listener in init()
            } else if (e.key === 'l' && e.ctrlKey) {
                // Ctrl+L: Redraw screen
                e.preventDefault();
                renderOutput();
            } else if (e.key === 'PageUp') {
                // PageUp: Scroll output up (triggers pause via scroll handler)
                e.preventDefault();
                elements.outputContainer.scrollBy(0, -elements.outputContainer.clientHeight);
            } else if (e.key === 'PageDown') {
                // PageDown: Scroll output down
                e.preventDefault();
                elements.outputContainer.scrollBy(0, elements.outputContainer.clientHeight);
                // If at bottom now, unpause or release pending (same as Tab)
                if (isAtBottom()) {
                    const world = worlds[currentWorldIndex];
                    const serverPending = world ? (world.pending_count || 0) : 0;
                    if (pendingLines.length === 0 && serverPending === 0) {
                        paused = false;
                        linesSincePause = 0;
                        updateStatusBar();
                    } else {
                        // Release one screenful when at bottom with pending (same as Tab)
                        releaseScreenful();
                    }
                }
            }
            // Note: F2, F3, F4 are handled at document level (before this handler)
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

        // Connection error modal buttons
        elements.connectionRetryBtn.onclick = function() {
            hideConnectionErrorModal();
            connectionFailures = 0;
            // Keep using ws:// fallback if it worked, otherwise reset to try wss:// again
            // (user can refresh page to fully reset)
            triedWsFallback = false;
            connect();
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
            // Reconnect and resend the command after authentication
            connectionFailures = 0;
            triedWsFallback = false;
            connect();
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
        elements.setupCloseBtn.onclick = closeSetupPopup;
        elements.setupMoreModeToggle.onclick = function() {
            setupMoreMode = !setupMoreMode;
            updateSetupPopupUI();
        };
        // Note: show tags removed from setup - controlled by F2 or /tag command
        elements.setupAnsiMusicToggle.onclick = function() {
            setupAnsiMusic = !setupAnsiMusic;
            updateSetupPopupUI();
        };
        elements.setupTlsProxyToggle.onclick = function() {
            setupTlsProxy = !setupTlsProxy;
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
        elements.setupSaveBtn.onclick = saveSetupSettings;
        elements.setupCancelBtn.onclick = closeSetupPopup;

        // Web settings popup (use edit state, not global state)
        elements.webProtocolSelect.onchange = function() {
            editWebSecure = this.value === 'secure';
            updateWebPopupUI();
        };
        elements.webHttpEnabledSelect.onchange = function() {
            editHttpEnabled = this.value === 'on';
            updateWebPopupUI();
        };
        elements.webWsEnabledSelect.onchange = function() {
            editWsEnabled = this.value === 'on';
            updateWebPopupUI();
        };
        elements.webSaveBtn.onclick = saveWebSettings;
        elements.webCancelBtn.onclick = closeWebPopup;
        elements.webCloseBtn.onclick = closeWebPopup;

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
                    connectionFailures = 0;
                    connect();
                } else if (ws.readyState === WebSocket.OPEN && authenticated) {
                    // Socket looks open - verify with a ping
                    try {
                        ws.send(JSON.stringify({ type: 'Ping' }));
                    } catch (e) {
                        // Send failed, connection is dead
                        ws.close();
                        connectionFailures = 0;
                        connect();
                        return;
                    }
                    // Wait up to 3 seconds for Pong response
                    wakePongTimeout = setTimeout(function() {
                        wakePongTimeout = null;
                        // No pong received - connection is stale
                        if (ws) {
                            ws.close();
                        }
                        connectionFailures = 0;
                        connect();
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
            const certUrl = `https://${host}:${window.WS_PORT}/`;
            warning.innerHTML = `
                <div style="margin-bottom:10px;font-weight:bold;">WebSocket Connection Failed</div>
                <div style="margin-bottom:10px;">If using a self-signed certificate, you need to accept it for the WebSocket port.</div>
                <a href="${certUrl}" target="_blank" style="color:#fff;text-decoration:underline;">Click here to accept the certificate for port ${window.WS_PORT}</a>
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

    // Expose native WebSocket check for debugging
    window.isUsingNativeWebSocket = function() {
        return usingNativeWebSocket;
    };

    // Start the app
    init();
})();
