// Clay MUD Client - Web Interface

(function() {
    'use strict';

    // DOM elements
    const elements = {
        output: document.getElementById('output'),
        outputContainer: document.getElementById('output-container'),
        statusIndicator: document.getElementById('status-indicator'),
        worldName: document.getElementById('world-name'),
        activityIndicator: document.getElementById('activity-indicator'),
        separatorFill: document.getElementById('separator-fill'),
        statusTime: document.getElementById('status-time'),
        prompt: document.getElementById('prompt'),
        input: document.getElementById('input'),
        sendBtn: document.getElementById('send-btn'),
        authModal: document.getElementById('auth-modal'),
        authPassword: document.getElementById('auth-password'),
        authError: document.getElementById('auth-error'),
        authSubmit: document.getElementById('auth-submit'),
        connectingOverlay: document.getElementById('connecting-overlay'),
        // Toolbar (desktop)
        toolbar: document.getElementById('toolbar'),
        menuBtn: document.getElementById('menu-btn'),
        menuDropdown: document.getElementById('menu-dropdown'),
        fontSmall: document.getElementById('font-small'),
        fontMedium: document.getElementById('font-medium'),
        fontLarge: document.getElementById('font-large'),
        // Mobile toolbar
        mobileToolbar: document.getElementById('mobile-toolbar'),
        mobileMenuBtn: document.getElementById('mobile-menu-btn'),
        mobileMenuDropdown: document.getElementById('mobile-menu-dropdown'),
        mobilePgUpBtn: document.getElementById('mobile-pgup-btn'),
        mobileUpBtn: document.getElementById('mobile-up-btn'),
        mobileDownBtn: document.getElementById('mobile-down-btn'),
        mobilePgDnBtn: document.getElementById('mobile-pgdn-btn'),
        // Actions List popup
        actionsListModal: document.getElementById('actions-list-modal'),
        actionsList: document.getElementById('actions-list'),
        actionAddBtn: document.getElementById('action-add-btn'),
        actionEditBtn: document.getElementById('action-edit-btn'),
        actionDeleteBtn: document.getElementById('action-delete-btn'),
        actionCancelBtn: document.getElementById('action-cancel-btn'),
        // Actions Editor popup
        actionsEditorModal: document.getElementById('actions-editor-modal'),
        actionEditorTitle: document.getElementById('action-editor-title'),
        actionName: document.getElementById('action-name'),
        actionWorld: document.getElementById('action-world'),
        actionPattern: document.getElementById('action-pattern'),
        actionCommand: document.getElementById('action-command'),
        actionError: document.getElementById('action-error'),
        actionSaveBtn: document.getElementById('action-save-btn'),
        actionEditorCancelBtn: document.getElementById('action-editor-cancel-btn'),
        // Actions Confirm Delete popup
        actionConfirmModal: document.getElementById('action-confirm-modal'),
        actionConfirmText: document.getElementById('action-confirm-text'),
        actionConfirmYesBtn: document.getElementById('action-confirm-yes-btn'),
        actionConfirmNoBtn: document.getElementById('action-confirm-no-btn'),
        // Worlds list popup
        worldsModal: document.getElementById('worlds-modal'),
        worldsTableBody: document.getElementById('worlds-table-body'),
        worldsCloseBtn: document.getElementById('worlds-close-btn'),
        // World selector popup
        worldSelectorModal: document.getElementById('world-selector-modal'),
        worldFilter: document.getElementById('world-filter'),
        worldSelectorList: document.getElementById('world-selector-list'),
        worldEditBtn: document.getElementById('world-edit-btn'),
        worldConnectBtn: document.getElementById('world-connect-btn'),
        worldSwitchBtn: document.getElementById('world-switch-btn'),
        worldSelectorCancelBtn: document.getElementById('world-selector-cancel-btn')
    };

    // State
    let ws = null;
    let authenticated = false;
    let worlds = [];
    let currentWorldIndex = 0;
    let commandHistory = [];
    let historyIndex = -1;
    let connectionFailures = 0;
    let inputHeight = 1;

    // Cached rendered output per world (array of DOM elements)
    let worldOutputCache = [];

    // More-mode state (per world)
    let moreModeEnabled = true;
    let paused = false;
    let pendingLines = [];
    let linesSincePause = 0;

    // Settings
    let worldSwitchMode = 'Unseen First';  // 'Unseen First' or 'Alphabetical'

    // Actions state
    let actions = [];
    let actionsListPopupOpen = false;
    let actionsEditorPopupOpen = false;
    let actionsConfirmPopupOpen = false;
    let selectedActionIndex = -1;
    let editingActionIndex = -1;  // -1 = new action, >=0 = editing existing

    // Tag display state
    let showTags = false;

    // World popup state
    let worldsPopupOpen = false;
    let worldSelectorPopupOpen = false;
    let selectedWorldIndex = -1;
    let selectedWorldsRowIndex = -1; // For worlds list popup (/worlds)

    // Menu state
    let menuOpen = false;
    let mobileMenuOpen = false;

    // Font size state: 'small' (11px), 'medium' (14px), 'large' (18px)
    let currentFontSize = 'medium';
    const fontSizes = {
        small: 11,   // Phone
        medium: 14,  // Tablet
        large: 18    // Desktop
    };

    // Device mode: 'desktop' or 'mobile'
    let deviceMode = 'desktop';

    // Detect device type and return appropriate font size
    function detectDeviceType() {
        const width = window.innerWidth;
        const hasTouch = 'ontouchstart' in window || navigator.maxTouchPoints > 0;

        // Phone: narrow screen (< 768px)
        if (width < 768) {
            return { fontSize: 'small', mode: 'mobile' };
        }
        // Tablet: medium screen with touch (768-1024px)
        if (width <= 1024 && hasTouch) {
            return { fontSize: 'medium', mode: 'mobile' };
        }
        // Desktop: wide screen or no touch
        return { fontSize: 'large', mode: 'desktop' };
    }

    // Setup toolbars based on device mode
    function setupToolbars(mode) {
        deviceMode = mode;
        if (mode === 'mobile') {
            // Hide desktop toolbar, show mobile toolbar
            elements.toolbar.style.display = 'none';
            elements.mobileToolbar.classList.add('visible');
            // Remove top padding since no fixed toolbar
            elements.outputContainer.style.paddingTop = '2px';
        } else {
            // Show desktop toolbar, hide mobile toolbar
            elements.toolbar.style.display = 'flex';
            elements.mobileToolbar.classList.remove('visible');
            // Add padding for fixed toolbar
            elements.outputContainer.style.paddingTop = '40px';
        }
    }

    // Initialize
    function init() {
        // Detect device type and configure UI
        const device = detectDeviceType();
        setFontSize(device.fontSize);
        setupToolbars(device.mode);

        setupEventListeners();
        connect();
        updateTime();
        setInterval(updateTime, 1000);
    }

    // Get visible line count in output area
    function getVisibleLineCount() {
        const fontSize = fontSizes[currentFontSize] || 14;
        const lineHeight = fontSize * 1.2; // font-size * line-height
        return Math.floor(elements.outputContainer.clientHeight / lineHeight);
    }

    // Check if scrolled to bottom
    function isAtBottom() {
        const container = elements.outputContainer;
        return container.scrollHeight - container.scrollTop <= container.clientHeight + 5;
    }

    // Connect to WebSocket server
    function connect() {
        showConnecting(true);

        const host = window.location.hostname;
        const wsUrl = `${window.WS_PROTOCOL}://${host}:${window.WS_PORT}`;

        try {
            ws = new WebSocket(wsUrl);

            ws.onopen = function() {
                connectionFailures = 0;
                hideCertWarning();
                showConnecting(false);
                showAuthModal(true);
                elements.authPassword.focus();
            };

            ws.onclose = function() {
                authenticated = false;
                showConnecting(false);
                connectionFailures++;
                // If using wss:// and connection keeps failing, show certificate warning
                if (window.WS_PROTOCOL === 'wss' && connectionFailures >= 2) {
                    showCertWarning();
                }
                setTimeout(connect, 3000); // Reconnect after 3 seconds
            };

            ws.onerror = function() {
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
            case 'AuthResponse':
                if (msg.success) {
                    authenticated = true;
                    showAuthModal(false);
                    elements.authError.textContent = '';
                } else {
                    elements.authError.textContent = msg.error || 'Authentication failed';
                    elements.authPassword.value = '';
                    elements.authPassword.focus();
                }
                break;

            case 'InitialState':
                worlds = msg.worlds || [];
                currentWorldIndex = msg.current_world_index || 0;
                actions = msg.actions || [];
                // Initialize output cache for each world (empty - will be populated on render)
                worldOutputCache = worlds.map(() => []);
                // Ensure output_lines arrays exist
                worlds.forEach((world) => {
                    if (!world.output_lines) {
                        world.output_lines = [];
                    }
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
                    if (msg.settings.world_switch_mode !== undefined) {
                        worldSwitchMode = msg.settings.world_switch_mode;
                    }
                }
                renderOutput();
                updateStatusBar();
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
                        // Split by any line ending
                        const rawLines = msg.data.split(/\r\n|\n|\r/);
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
                            world.output_lines.push(line);
                            if (msg.world_index === currentWorldIndex) {
                                handleIncomingLine(line, msg.world_index, lineIndex);
                            } else {
                                world.unseen_lines = (world.unseen_lines || 0) + 1;
                            }
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
                    updateStatusBar();
                }
                break;

            case 'WorldDisconnected':
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].connected = false;
                    updateStatusBar();
                }
                break;

            case 'WorldSwitched':
                // Console switched worlds - we ignore this to maintain independent view
                // Web interface tracks its own current world separately
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
                    if (msg.settings.world_switch_mode !== undefined) {
                        worldSwitchMode = msg.settings.world_switch_mode;
                    }
                }
                break;

            case 'Pong':
                // Keepalive response
                break;

            case 'ActionsUpdated':
                actions = msg.actions || [];
                if (actionsListPopupOpen) {
                    renderActionsList();
                }
                break;

            case 'UnseenCleared':
                // Another client (console, web, or GUI) has viewed this world
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].unseen_lines = 0;
                    updateStatusBar();
                }
                break;

            default:
                console.log('Unknown message type:', msg.type);
        }
    }

    // Handle incoming line with more-mode logic
    function handleIncomingLine(text, worldIndex, lineIndex) {
        if (!text) return;

        const visibleLines = getVisibleLineCount();
        const threshold = Math.max(1, visibleLines - 2);

        if (paused) {
            // Already paused, queue the line info
            pendingLines.push({ text, worldIndex, lineIndex });
            updateStatusBar();
        } else if (moreModeEnabled && linesSincePause >= threshold) {
            // Trigger pause
            paused = true;
            pendingLines.push({ text, worldIndex, lineIndex });
            // Scroll to bottom to show what we have so far
            scrollToBottom();
            updateStatusBar();
        } else {
            // Normal display - append the line
            linesSincePause++;
            appendNewLine(text, worldIndex, lineIndex);
        }
    }

    // Release one screenful of pending lines
    function releaseScreenful() {
        if (!paused || pendingLines.length === 0) return;

        const count = Math.max(1, getVisibleLineCount() - 2);
        const toRelease = pendingLines.splice(0, count);

        toRelease.forEach(item => {
            appendNewLine(item.text, item.worldIndex, item.lineIndex);
        });

        if (pendingLines.length === 0) {
            paused = false;
            linesSincePause = 0;
        }

        updateStatusBar();
    }

    // Release all pending lines
    function releaseAll() {
        if (!paused) return;

        pendingLines.forEach(item => {
            appendNewLine(item.text, item.worldIndex, item.lineIndex);
        });
        pendingLines = [];
        paused = false;
        linesSincePause = 0;

        updateStatusBar();
    }

    // Send message to server
    function send(msg) {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify(msg));
        }
    }

    // Authenticate
    function authenticate() {
        const password = elements.authPassword.value;
        if (!password) return;

        // Hash password with SHA-256
        hashPassword(password).then(hash => {
            send({ type: 'AuthRequest', password_hash: hash });
        });
    }

    // SHA-256 hash
    async function hashPassword(password) {
        const encoder = new TextEncoder();
        const data = encoder.encode(password);
        const hashBuffer = await crypto.subtle.digest('SHA-256', data);
        const hashArray = Array.from(new Uint8Array(hashBuffer));
        return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
    }

    // Send command
    function sendCommand() {
        const cmd = elements.input.value;
        if (cmd.length === 0 && !authenticated) return;

        const trimmedCmd = cmd.trim();

        // Check for local commands
        if (trimmedCmd === '/actions') {
            elements.input.value = '';
            openActionsPopup();
            return;
        }

        // /worlds or /l - show worlds list popup
        if (trimmedCmd === '/worlds' || trimmedCmd === '/l') {
            elements.input.value = '';
            openWorldsPopup();
            return;
        }

        // /world (no args) - show world selector popup
        if (trimmedCmd === '/world') {
            elements.input.value = '';
            openWorldSelectorPopup();
            return;
        }

        // /world <name> - switch to or connect to named world
        if (trimmedCmd.startsWith('/world ')) {
            const worldName = trimmedCmd.substring(7).trim();
            if (worldName) {
                elements.input.value = '';
                handleWorldCommand(worldName);
                return;
            }
        }

        // Release all pending lines when sending a command
        if (paused) {
            releaseAll();
        }

        // Reset lines since pause counter on user input
        linesSincePause = 0;

        send({
            type: 'SendCommand',
            world_index: currentWorldIndex,
            command: cmd
        });

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

    // Switch world locally (does not affect console)
    function switchWorldLocal(index) {
        if (index >= 0 && index < worlds.length && index !== currentWorldIndex) {
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
        }
    }

    // Render output - render all lines as text with line breaks
    function renderOutput() {
        elements.output.innerHTML = '';

        const world = worlds[currentWorldIndex];
        if (!world) return;

        const lines = world.output_lines || [];

        // Build all lines as HTML with explicit <br> line breaks
        const htmlParts = [];
        for (let i = 0; i < lines.length; i++) {
            const rawLine = lines[i];
            if (rawLine === undefined || rawLine === null) continue;

            // Strip newlines/carriage returns
            const cleanLine = String(rawLine).replace(/[\r\n]+/g, '');

            const displayText = showTags ? cleanLine : stripMudTag(cleanLine);
            const html = convertDiscordEmojis(linkifyUrls(parseAnsi(displayText)));
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
        const displayText = showTags ? text : stripMudTag(text);
        const html = convertDiscordEmojis(linkifyUrls(parseAnsi(displayText)));
        worldOutputCache[worldIndex][lineIndex] = { html, showTags };
        return html;
    }

    // Append a new line to current world's output (already visible)
    function appendNewLine(text, worldIndex, lineIndex) {
        // Strip newlines/carriage returns
        const cleanText = String(text).replace(/[\r\n]+/g, '');

        const displayText = showTags ? cleanText : stripMudTag(cleanText);
        const html = convertDiscordEmojis(linkifyUrls(parseAnsi(displayText)));

        // Append to output with a <br> prefix (if not first line)
        if (elements.output.innerHTML.length > 0) {
            elements.output.innerHTML += '<br>' + html;
        } else {
            elements.output.innerHTML = html;
        }

        scrollToBottom();
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

        // 256-color palette (first 16 are standard, 16-231 are RGB cube, 232-255 are grayscale)
        function color256ToRgb(n) {
            if (n < 16) {
                // Standard colors
                const standard = [
                    [0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0],
                    [0, 0, 205], [205, 0, 205], [0, 205, 205], [192, 192, 192],
                    [128, 128, 128], [255, 0, 0], [0, 255, 0], [255, 255, 0],
                    [0, 0, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255]
                ];
                return standard[n];
            } else if (n < 232) {
                // 216 color cube (6x6x6)
                n -= 16;
                const r = Math.floor(n / 36) * 51;
                const g = Math.floor((n % 36) / 6) * 51;
                const b = (n % 6) * 51;
                return [r, g, b];
            } else {
                // Grayscale (24 shades)
                const gray = (n - 232) * 10 + 8;
                return [gray, gray, gray];
            }
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
                const textBefore = escapeHtml(text.substring(lastIndex, match.index));
                const classes = currentClasses.length > 0 ? ` class="${currentClasses.join(' ')}"` : '';
                const styles = (currentFgStyle || currentBgStyle) ? ` style="${currentFgStyle}${currentBgStyle}"` : '';
                if (classes || styles) {
                    result += `<span${classes}${styles}>${textBefore}</span>`;
                } else {
                    result += textBefore;
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
                } else if (code === 3) {
                    currentClasses.push('ansi-italic');
                } else if (code === 4) {
                    currentClasses.push('ansi-underline');
                } else if (code >= 30 && code <= 37) {
                    // Basic foreground colors
                    currentClasses = currentClasses.filter(c => !c.startsWith('ansi-') || c.startsWith('ansi-bg-') || c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline');
                    currentFgStyle = '';
                    const colors = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
                    currentClasses.push('ansi-' + colors[code - 30]);
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
            const classes = currentClasses.length > 0 ? ` class="${currentClasses.join(' ')}"` : '';
            const styles = (currentFgStyle || currentBgStyle) ? ` style="${currentFgStyle}${currentBgStyle}"` : '';
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

    // Strip ANSI escape codes from text
    function stripAnsi(text) {
        if (!text) return text;
        // Remove all ANSI CSI sequences
        return text.replace(/\x1b\[[0-9;]*[A-Za-z@`~]/g, '').replace(/[\x00-\x1f]/g, '');
    }

    // Linkify URLs in HTML text (after ANSI parsing)
    // Matches http://, https://, and www. URLs
    function linkifyUrls(html) {
        // URL pattern that works on HTML-escaped text
        // Matches http://, https://, or www. followed by non-whitespace
        // Stops at HTML tags, quotes, or common punctuation at end
        const urlPattern = /(\b(?:https?:\/\/|www\.)[^\s<>"']*[^\s<>"'.,;:!?\)\]}>])/gi;

        return html.replace(urlPattern, function(url) {
            // Add protocol if missing (for www. URLs)
            const href = url.startsWith('www.') ? 'https://' + url : url;
            return `<a href="${href}" target="_blank" rel="noopener" class="output-link">${url}</a>`;
        });
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

    // Scroll to bottom
    function scrollToBottom() {
        elements.outputContainer.scrollTop = elements.outputContainer.scrollHeight;
    }

    // Format count for status indicator (matches console behavior)
    function formatCount(n) {
        if (n >= 1000000) return 'Alot';
        if (n >= 10000) return ' ' + Math.floor(n / 1000) + 'K';
        return n.toString().padStart(4, ' ');
    }

    // Update status bar (console-style with underscores)
    function updateStatusBar() {
        const world = worlds[currentWorldIndex];

        // Status indicator: shows More/Hist when active, underscores when idle
        if (paused && pendingLines.length > 0) {
            elements.statusIndicator.textContent = 'More:' + formatCount(pendingLines.length);
            elements.statusIndicator.className = 'paused';
        } else if (!isAtBottom()) {
            // Calculate lines from bottom
            const container = elements.outputContainer;
            const fontSize = fontSizes[currentFontSize] || 14;
            const lineHeight = fontSize * 1.2;
            const linesFromBottom = Math.floor((container.scrollHeight - container.scrollTop - container.clientHeight) / lineHeight);
            elements.statusIndicator.textContent = 'Hist:' + formatCount(linesFromBottom);
            elements.statusIndicator.className = 'scrolled';
        } else {
            elements.statusIndicator.textContent = '___________';
            elements.statusIndicator.className = '';
        }

        if (world) {
            elements.worldName.textContent = ' ' + (world.name || '');
        }

        // Activity indicator
        let unseenCount = 0;
        worlds.forEach((w, i) => {
            if (i !== currentWorldIndex && w.unseen_lines > 0) {
                unseenCount++;
            }
        });
        elements.activityIndicator.textContent = unseenCount > 0 ? ` (Activity: ${unseenCount})` : '';

        // Fill remaining space with underscores
        // Calculate how many underscores fit based on container width and font size
        const fillWidth = elements.separatorFill.offsetWidth || window.innerWidth;
        const fontSize = fontSizes[currentFontSize] || 14;
        const charWidth = fontSize * 0.6; // Approximate width ratio for monospace
        const numUnderscores = Math.ceil(fillWidth / charWidth) + 20; // Add buffer
        elements.separatorFill.textContent = '_'.repeat(Math.max(200, numUnderscores));
    }

    // Update time
    function updateTime() {
        const now = new Date();
        const hours = now.getHours().toString().padStart(2, '0');
        const minutes = now.getMinutes().toString().padStart(2, '0');
        elements.statusTime.textContent = `${hours}:${minutes}`;
    }

    // Set input area height (number of lines)
    function setInputHeight(lines) {
        inputHeight = Math.max(1, Math.min(15, lines));
        const fontSize = fontSizes[currentFontSize] || 14;
        const lineHeight = 1.2 * fontSize; // line-height * font-size
        elements.input.style.height = (inputHeight * lineHeight) + 'px';
        elements.input.rows = inputHeight;
    }

    // Show/hide connecting overlay
    function showConnecting(show) {
        elements.connectingOverlay.className = 'overlay' + (show ? ' visible' : '');
    }

    // Show/hide auth modal
    function showAuthModal(show) {
        elements.authModal.className = 'modal' + (show ? ' visible' : '');
        if (show) {
            elements.authPassword.value = '';
            elements.authError.textContent = '';
        }
    }

    // Actions popup functions (split into List and Editor)

    // Open Actions List popup
    function openActionsListPopup() {
        actionsListPopupOpen = true;
        selectedActionIndex = actions.length > 0 ? 0 : -1;
        elements.actionsListModal.className = 'modal visible';
        renderActionsList();
    }

    // Close Actions List popup
    function closeActionsListPopup() {
        actionsListPopupOpen = false;
        elements.actionsListModal.className = 'modal';
        elements.input.focus();
    }

    // Render actions list with Name, World, Pattern columns
    function renderActionsList() {
        elements.actionsList.innerHTML = '';
        if (actions.length === 0) {
            const div = document.createElement('div');
            div.style.padding = '8px';
            div.style.color = '#888';
            div.textContent = 'No actions defined.';
            elements.actionsList.appendChild(div);
            return;
        }
        actions.forEach((action, index) => {
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
            elements.actionPattern.value = action.pattern || '';
            elements.actionCommand.value = action.command || '';
        } else {
            // New action
            elements.actionEditorTitle.textContent = 'New Action';
            elements.actionName.value = '';
            elements.actionWorld.value = '';
            elements.actionPattern.value = '';
            elements.actionCommand.value = '';
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
            pattern: elements.actionPattern.value,
            command: elements.actionCommand.value
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

    // Worlds list popup functions (/worlds, /l)
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

            // Send/Recv
            const tdSendRecv = document.createElement('td');
            tdSendRecv.textContent = formatElapsed(world.last_send_secs) + '/' + formatElapsed(world.last_recv_secs);
            tr.appendChild(tdSendRecv);

            // KeepAlive type
            const tdKA = document.createElement('td');
            tdKA.textContent = world.keep_alive_type || 'NOP';
            tr.appendChild(tdKA);

            // Last/Next KA
            const tdLastNext = document.createElement('td');
            tdLastNext.textContent = formatElapsed(world.last_nop_secs) + '/' + formatNextKA(world.last_send_secs, world.last_recv_secs);
            tr.appendChild(tdLastNext);

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

    // World selector popup functions (/world)
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
        elements.worldSelectorList.innerHTML = '';

        worlds.forEach((world, index) => {
            // Filter by name, hostname, or user
            const name = (world.name || '').toLowerCase();
            const hostname = (world.settings?.hostname || '').toLowerCase();
            const user = (world.settings?.user || '').toLowerCase();

            if (filter && !name.includes(filter) && !hostname.includes(filter) && !user.includes(filter)) {
                return; // Skip non-matching worlds
            }

            const div = document.createElement('div');
            div.className = 'world-selector-item';
            if (index === selectedWorldIndex) div.className += ' selected';
            if (index === currentWorldIndex) div.className += ' current';

            // World info (name + host)
            const infoDiv = document.createElement('div');
            infoDiv.className = 'world-info';

            const nameSpan = document.createElement('span');
            nameSpan.className = 'world-name';
            nameSpan.textContent = stripAnsi(world.name || '(unnamed)').trim();
            infoDiv.appendChild(nameSpan);

            if (world.settings?.hostname) {
                const hostSpan = document.createElement('span');
                hostSpan.className = 'world-host';
                hostSpan.textContent = world.settings.hostname + (world.settings.port ? ':' + world.settings.port : '');
                infoDiv.appendChild(hostSpan);
            }

            div.appendChild(infoDiv);

            // Status indicator
            const statusSpan = document.createElement('span');
            statusSpan.className = 'world-status ' + (world.connected ? 'connected' : 'disconnected');
            statusSpan.textContent = world.connected ? '●' : '○';
            div.appendChild(statusSpan);

            div.onclick = () => selectWorld(index);
            div.ondblclick = () => {
                selectWorld(index);
                switchToSelectedWorld();
            };

            elements.worldSelectorList.appendChild(div);
        });
    }

    function selectWorld(index) {
        selectedWorldIndex = index;
        renderWorldSelectorList();
        scrollSelectedWorldIntoView();
    }

    // Scroll the selected world into view in world selector list
    function scrollSelectedWorldIntoView() {
        requestAnimationFrame(() => {
            const container = elements.worldSelectorList;
            const selectedItem = container?.querySelector('.world-selector-item.selected');
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

    // Get indices of worlds that match the current filter
    function getFilteredWorldIndices() {
        const filter = elements.worldFilter.value.toLowerCase();
        const indices = [];
        worlds.forEach((world, index) => {
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
            // Send connect command to server
            send({
                type: 'ConnectWorld',
                world_index: selectedWorldIndex
            });
            closeWorldSelectorPopup();
        }
    }

    function editSelectedWorld() {
        if (selectedWorldIndex >= 0 && selectedWorldIndex < worlds.length) {
            const world = worlds[selectedWorldIndex];
            // Send command to open world editor on server
            send({
                type: 'SendCommand',
                command: '/world -e ' + world.name,
                world_index: currentWorldIndex
            });
            closeWorldSelectorPopup();
        }
    }

    // Handle /world <name> command
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
            // If not connected, connect
            if (!world.connected) {
                send({
                    type: 'ConnectWorld',
                    world_index: worldIndex
                });
            }
        } else {
            // World not found - send command to server to create/handle it
            send({
                type: 'SendCommand',
                world_index: currentWorldIndex,
                command: '/world ' + worldName
            });
        }
    }

    // Check if any popup is open
    function isAnyPopupOpen() {
        return actionsListPopupOpen || actionsEditorPopupOpen || actionsConfirmPopupOpen || worldsPopupOpen || worldSelectorPopupOpen;
    }

    // Check if a world should be included in cycling (connected OR has unseen output)
    function isWorldActive(world) {
        return world.connected || (world.unseen_lines && world.unseen_lines > 0);
    }

    // Check if a world has unseen output (for pending_first prioritization)
    function worldHasPending(world) {
        return world.unseen_lines && world.unseen_lines > 0;
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

    // Get next active world index (cycling forward)
    function getNextActiveWorld() {
        const activeIndices = getActiveWorldIndices();
        if (activeIndices.length <= 1) return currentWorldIndex;

        // If worldSwitchMode is 'Unseen First', check for OTHER worlds with unseen output first
        if (worldSwitchMode === 'Unseen First') {
            const unseenWorlds = activeIndices
                .filter(i => i !== currentWorldIndex && worldHasPending(worlds[i]))
                .sort((a, b) => (worlds[a].name || '').toLowerCase().localeCompare((worlds[b].name || '').toLowerCase()));

            if (unseenWorlds.length > 0) {
                return unseenWorlds[0]; // Go to first world with unseen output
            }
        }

        // Fall back to alphabetical cycling
        const currentPos = activeIndices.indexOf(currentWorldIndex);
        if (currentPos === -1) {
            return activeIndices[0];
        }
        return activeIndices[(currentPos + 1) % activeIndices.length];
    }

    // Get previous active world index (cycling backward)
    function getPrevActiveWorld() {
        const activeIndices = getActiveWorldIndices();
        if (activeIndices.length <= 1) return currentWorldIndex;

        // If worldSwitchMode is 'Unseen First', check for OTHER worlds with unseen output first
        if (worldSwitchMode === 'Unseen First') {
            const unseenWorlds = activeIndices
                .filter(i => i !== currentWorldIndex && worldHasPending(worlds[i]))
                .sort((a, b) => (worlds[a].name || '').toLowerCase().localeCompare((worlds[b].name || '').toLowerCase()));

            if (unseenWorlds.length > 0) {
                return unseenWorlds[0]; // Go to first world with unseen output
            }
        }

        // Fall back to alphabetical cycling
        const currentPos = activeIndices.indexOf(currentWorldIndex);
        if (currentPos === -1) {
            return activeIndices[activeIndices.length - 1];
        }
        return activeIndices[(currentPos - 1 + activeIndices.length) % activeIndices.length];
    }

    // Toggle hamburger menu (desktop)
    function toggleMenu() {
        menuOpen = !menuOpen;
        elements.menuDropdown.className = 'dropdown' + (menuOpen ? ' visible' : '');
    }

    // Close hamburger menu (desktop)
    function closeMenu() {
        menuOpen = false;
        elements.menuDropdown.className = 'dropdown';
    }

    // Toggle mobile menu
    function toggleMobileMenu() {
        mobileMenuOpen = !mobileMenuOpen;
        elements.mobileMenuDropdown.className = 'dropdown' + (mobileMenuOpen ? ' visible' : '');
    }

    // Close mobile menu
    function closeMobileMenu() {
        mobileMenuOpen = false;
        elements.mobileMenuDropdown.className = 'dropdown';
    }

    // Handle menu item click
    function handleMenuItem(action) {
        closeMenu();
        closeMobileMenu();
        switch (action) {
            case 'worlds':
                openWorldsPopup();
                break;
            case 'world-selector':
                openWorldSelectorPopup();
                break;
            case 'actions':
                openActionsPopup();
                break;
            case 'toggle-tags':
                showTags = !showTags;
                renderOutput();
                break;
        }
    }

    // Set font size
    function setFontSize(size) {
        currentFontSize = size;
        const px = fontSizes[size];

        // Update body font size
        document.body.style.fontSize = px + 'px';

        // Update active button
        elements.fontSmall.className = 'font-btn' + (size === 'small' ? ' active' : '');
        elements.fontMedium.className = 'font-btn' + (size === 'medium' ? ' active' : '');
        elements.fontLarge.className = 'font-btn' + (size === 'large' ? ' active' : '');

        // Re-render to update line height calculations
        updateStatusBar();
    }

    // Setup event listeners
    function setupEventListeners() {
        // Send button
        elements.sendBtn.onclick = sendCommand;

        // Hamburger menu
        elements.menuBtn.onclick = function(e) {
            e.stopPropagation();
            toggleMenu();
        };

        // Menu items
        elements.menuDropdown.onclick = function(e) {
            e.stopPropagation();
            const item = e.target.closest('.dropdown-item');
            if (item) {
                handleMenuItem(item.dataset.action);
            }
        };

        // Font size buttons
        elements.fontSmall.onclick = function(e) {
            e.stopPropagation();
            setFontSize('small');
        };
        elements.fontMedium.onclick = function(e) {
            e.stopPropagation();
            setFontSize('medium');
        };
        elements.fontLarge.onclick = function(e) {
            e.stopPropagation();
            setFontSize('large');
        };

        // Mobile toolbar buttons
        elements.mobileMenuBtn.onclick = function(e) {
            e.stopPropagation();
            toggleMobileMenu();
        };

        elements.mobileMenuDropdown.onclick = function(e) {
            e.stopPropagation();
            const item = e.target.closest('.dropdown-item');
            if (item) {
                handleMenuItem(item.dataset.action);
            }
        };

        elements.mobileUpBtn.onclick = function(e) {
            e.preventDefault();
            e.stopPropagation();
            // Cycle to previous active world
            const prevIndex = getPrevActiveWorld();
            if (prevIndex !== currentWorldIndex) {
                switchWorldLocal(prevIndex);
            }
            elements.input.focus();
        };

        elements.mobileDownBtn.onclick = function(e) {
            e.preventDefault();
            e.stopPropagation();
            // Cycle to next active world
            const nextIndex = getNextActiveWorld();
            if (nextIndex !== currentWorldIndex) {
                switchWorldLocal(nextIndex);
            }
            elements.input.focus();
        };

        elements.mobilePgUpBtn.onclick = function(e) {
            e.preventDefault();
            e.stopPropagation();
            // Page up - scroll output
            const container = elements.outputContainer;
            const pageHeight = container.clientHeight * 0.9;
            container.scrollTop = Math.max(0, container.scrollTop - pageHeight);
            updateStatusBar();
            elements.input.focus();
        };

        elements.mobilePgDnBtn.onclick = function(e) {
            e.preventDefault();
            e.stopPropagation();
            // Page down - scroll output or release pending lines
            const container = elements.outputContainer;
            const world = worlds[currentWorldIndex];

            if (world && world.pendingLines && world.pendingLines.length > 0) {
                // Release one screenful of pending lines
                releasePendingLines(getVisibleLineCount());
            } else {
                // Just scroll down
                const pageHeight = container.clientHeight * 0.9;
                container.scrollTop += pageHeight;
            }
            updateStatusBar();
            elements.input.focus();
        };

        // Window resize handler to update separator fill
        window.addEventListener('resize', function() {
            updateStatusBar();
        });

        // Handle mobile keyboard visibility - keep toolbar at visual viewport top
        if (window.visualViewport) {
            const toolbar = document.getElementById('toolbar');
            window.visualViewport.addEventListener('resize', function() {
                // When keyboard appears, visualViewport height shrinks
                // Keep toolbar at the top of the visual viewport
                toolbar.style.top = window.visualViewport.offsetTop + 'px';
            });
            window.visualViewport.addEventListener('scroll', function() {
                toolbar.style.top = window.visualViewport.offsetTop + 'px';
            });
        }

        // Click anywhere to focus input and close menu
        document.body.onclick = function(e) {
            // Close menus if open
            if (menuOpen) {
                closeMenu();
            }
            if (mobileMenuOpen) {
                closeMobileMenu();
            }

            // Don't steal focus if user has selected text (for copy)
            const selection = window.getSelection();
            if (selection && selection.toString().length > 0) {
                return;
            }
            // Don't steal focus from modals or toolbars
            if (!elements.authModal.classList.contains('visible') &&
                !elements.actionsModal.classList.contains('visible') &&
                !elements.worldsModal.classList.contains('visible') &&
                !elements.worldSelectorModal.classList.contains('visible') &&
                !e.target.closest('#toolbar') &&
                !e.target.closest('#mobile-toolbar')) {
                elements.input.focus();
            }
        };

        // Scroll event to update status bar (for Hist indicator)
        elements.outputContainer.onscroll = function() {
            updateStatusBar();
            // If user scrolls up, trigger pause (like console behavior)
            if (moreModeEnabled && !paused && !isAtBottom()) {
                paused = true;
                updateStatusBar();
            }
            // If user scrolls to bottom, unpause
            if (paused && isAtBottom() && pendingLines.length === 0) {
                paused = false;
                linesSincePause = 0;
                updateStatusBar();
            }
        };

        // Document-level keyboard handler for navigation keys
        document.onkeydown = function(e) {
            // Skip if auth modal is visible
            if (elements.authModal.classList.contains('visible')) return;

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
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeActionsListPopup();
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
                if (paused && pendingLines.length > 0) {
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
                if (isAtBottom() && pendingLines.length === 0) {
                    paused = false;
                    linesSincePause = 0;
                    updateStatusBar();
                }
            } else if (e.key === 'ArrowUp' && !e.ctrlKey && document.activeElement !== elements.input) {
                // Up: Switch to previous active world (when not in input)
                e.preventDefault();
                const prevWorld = getPrevActiveWorld();
                if (prevWorld !== currentWorldIndex) {
                    switchWorldLocal(prevWorld);
                }
                elements.input.focus();
            } else if (e.key === 'ArrowDown' && !e.ctrlKey && document.activeElement !== elements.input) {
                // Down: Switch to next active world (when not in input)
                e.preventDefault();
                const nextWorld = getNextActiveWorld();
                if (nextWorld !== currentWorldIndex) {
                    switchWorldLocal(nextWorld);
                }
                elements.input.focus();
            } else if (e.key === 'F2') {
                // F2: Toggle MUD tag display
                e.preventDefault();
                showTags = !showTags;
                renderOutput();
            }
        };

        // Keyboard controls (console-style) - input-specific
        elements.input.addEventListener('keydown', function(e) {
            if (e.key === 'Enter' && !e.shiftKey) {
                // Send command (also releases all pending)
                e.preventDefault();
                e.stopPropagation();  // Prevent document-level handler from catching this
                sendCommand();
            } else if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey) {
                // Tab: Release one screenful of pending lines, or scroll down
                e.preventDefault(); // Always prevent default tab behavior
                if (paused && pendingLines.length > 0) {
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
            } else if (e.key === 'ArrowUp' && e.ctrlKey) {
                // Ctrl+Up: Increase input height
                e.preventDefault();
                if (inputHeight < 15) {
                    setInputHeight(inputHeight + 1);
                }
            } else if (e.key === 'ArrowDown' && e.ctrlKey) {
                // Ctrl+Down: Decrease input height
                e.preventDefault();
                if (inputHeight > 1) {
                    setInputHeight(inputHeight - 1);
                }
            } else if (e.key === 'ArrowUp' && !e.ctrlKey) {
                // Up: Switch to previous active world (local only, doesn't affect console)
                e.preventDefault();
                const prevWorld = getPrevActiveWorld();
                if (prevWorld !== currentWorldIndex) {
                    switchWorldLocal(prevWorld);
                }
            } else if (e.key === 'ArrowDown' && !e.ctrlKey) {
                // Down: Switch to next active world (local only, doesn't affect console)
                e.preventDefault();
                const nextWorld = getNextActiveWorld();
                if (nextWorld !== currentWorldIndex) {
                    switchWorldLocal(nextWorld);
                }
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
            } else if (e.key === 'w' && e.ctrlKey) {
                // Ctrl+W: Delete word before cursor
                e.preventDefault();
                const input = elements.input;
                const pos = input.selectionStart;
                const text = input.value;
                // Find start of word before cursor
                let start = pos;
                while (start > 0 && text[start - 1] === ' ') start--;
                while (start > 0 && text[start - 1] !== ' ') start--;
                input.value = text.substring(0, start) + text.substring(pos);
                input.selectionStart = input.selectionEnd = start;
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
                // If at bottom now and no pending, unpause
                if (isAtBottom() && pendingLines.length === 0) {
                    paused = false;
                    linesSincePause = 0;
                    updateStatusBar();
                }
            }
        });

        // Auth submit
        elements.authSubmit.onclick = authenticate;
        elements.authPassword.onkeydown = function(e) {
            if (e.key === 'Enter') {
                authenticate();
            }
        };

        // Actions List popup
        elements.actionAddBtn.onclick = () => openActionsEditorPopup(-1);
        elements.actionEditBtn.onclick = () => {
            if (selectedActionIndex >= 0 && selectedActionIndex < actions.length) {
                openActionsEditorPopup(selectedActionIndex);
            }
        };
        elements.actionDeleteBtn.onclick = openActionsConfirmPopup;
        elements.actionCancelBtn.onclick = closeActionsListPopup;

        // Actions Editor popup
        elements.actionSaveBtn.onclick = saveAction;
        elements.actionEditorCancelBtn.onclick = closeActionsEditorPopup;

        // Actions Confirm Delete popup
        elements.actionConfirmYesBtn.onclick = confirmDeleteAction;
        elements.actionConfirmNoBtn.onclick = closeActionsConfirmPopup;

        // Worlds list popup
        elements.worldsCloseBtn.onclick = closeWorldsPopup;

        // World selector popup
        elements.worldEditBtn.onclick = editSelectedWorld;
        elements.worldConnectBtn.onclick = connectSelectedWorld;
        elements.worldSwitchBtn.onclick = switchToSelectedWorld;
        elements.worldSelectorCancelBtn.onclick = closeWorldSelectorPopup;
        elements.worldFilter.oninput = function() {
            // Update selection if current selection is filtered out
            const visibleIndices = getFilteredWorldIndices();
            if (!visibleIndices.includes(selectedWorldIndex)) {
                selectedWorldIndex = visibleIndices.length > 0 ? visibleIndices[0] : -1;
            }
            renderWorldSelectorList();
        };

        // Keepalive ping every 30 seconds
        setInterval(function() {
            if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
                send({ type: 'Ping' });
            }
        }, 30000);
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

    // Start the app
    init();
})();
