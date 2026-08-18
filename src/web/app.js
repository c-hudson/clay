// Clay MUD Client - Web Interface

(function() {
    'use strict';

    // DIAGNOSTIC: surface uncaught errors on-screen (blank-screen debugging)
    function __clayShowError(msg) {
        try { if (window.Android && window.Android.showErrorBanner) window.Android.showErrorBanner(msg); } catch(_) {}
        try {
            var d = document.createElement('div');
            d.style.cssText = 'position:fixed;top:0;left:0;right:0;z-index:99999;background:#a00;color:#fff;font:14px monospace;padding:8px;white-space:pre-wrap;word-break:break-all;max-height:60vh;overflow:auto;';
            d.textContent = 'CLAY JS ERROR\n' + msg;
            (document.body || document.documentElement).appendChild(d);
        } catch(_) {}
    }
    window.addEventListener('error', function(e) {
        __clayShowError((e.message || 'error') + '\n@ ' + (e.filename || '?') + ':' + (e.lineno || '?') + ':' + (e.colno || '?') + (e.error && e.error.stack ? '\n' + e.error.stack : ''));
    });

    // Render a caught error as text. Both halves are needed: V8 (Android WebView, Chrome)
    // starts `stack` with "Name: message", but SpiderMonkey and JavaScriptCore (WebKitGTK,
    // i.e. the desktop GUI and Termux) put *only* the frames there - reporting `stack` alone
    // silently drops the single most useful line on those engines.
    function __clayErrText(e) {
        if (!e) return String(e);
        var head = e.message ? ((e.name ? e.name + ': ' : '') + e.message) : String(e);
        return e.stack ? head + '\n' + e.stack : head;
    }

    // Wrap an entry point so a throw inside it reports something usable.
    //
    // The global handler above is blind on Android: the app is loaded as
    // file:///android_asset/web/index.html, and under a file:// (opaque) origin the WebView
    // sanitizes uncaught errors from an external <script> to literally "Script error." with no
    // filename, line, or stack. A try/catch *inside* this script is not subject to that - it
    // receives the real Error - so guarding the entry points is what makes a failure
    // diagnosable at all on the platform where it matters most.
    //
    // Swallows after reporting: a broken click handler should leave a banner, not an unhandled
    // exception. Same pattern as the init() guard at the bottom of this file.
    function guard(name, fn) {
        return function() {
            try {
                return fn.apply(this, arguments);
            } catch (e) {
                __clayShowError(name + ' threw: ' + __clayErrText(e));
            }
        };
    }
    window.addEventListener('unhandledrejection', function(e) {
        var r = e.reason;
        __clayShowError('unhandled rejection: ' + (r && r.message ? r.message : String(r)) + (r && r.stack ? '\n' + r.stack : ''));
    });

    // IPC: send a message to the native Rust side.
    // Primary path: window.ipc.postMessage (wry-injected, uses webkit.messageHandlers).
    // Fallback: POST to clay://localhost/ipc — used when webkit.messageHandlers is
    // unavailable (e.g. Termux WebKit2GTK). Rust handles /ipc in the custom protocol handler.
    function sendIpc(msg) {
        if (window.ipc) {
            try {
                window.ipc.postMessage(msg);
                return;
            } catch (e) { /* fall through to protocol fallback */ }
        }
        if (window.WEBVIEW_MODE) {
            fetch('clay://localhost/ipc', { method: 'POST', body: msg }).catch(function() {});
        }
    }

    // Maximum line length to prevent performance issues with extremely long lines
    const MAX_LINE_LENGTH = 10000;

    // Truncate text if it exceeds MAX_LINE_LENGTH
    function truncateIfNeeded(text) {
        if (text.length > MAX_LINE_LENGTH) {
            return text.substring(0, MAX_LINE_LENGTH) + '\x1b[0m\x1b[33m... [truncated]\x1b[0m';
        }
        return text;
    }

    // --- Delivered-seq tracking (PROTOCOL-ROADMAP.md Phase C) ------------------------
    // `world._seenRanges` is a sorted, coalesced, non-overlapping array of INCLUSIVE seq
    // ranges [{start, end}, ...] recording every seq the server has actually DELIVERED to
    // us for that world. It is the single source of truth for three things that used to be
    // tracked separately and approximately:
    //
    //   world._max_seq          === last().end          (high-water mark)
    //   contiguousFrontier()    === ranges[0].end       (the resume/ack boundary)
    //   hasSeenSeq()            === set membership      (the dedup test)
    //
    // This replaces the old `_max_seq` + `_seqGaps` pair. `_max_seq` was a ONE-WAY RATCHET:
    // any batch whose seq was <= it got dropped whole unless it happened to overlap a gap
    // that had been *recorded*, and `_seqGaps` could only ever record "a batch skipped
    // ahead of the expected next seq". Neither could represent "lines I was never sent at
    // all", so a watermark that got ahead of the buffer - by ANY means, including a server
    // path not yet discovered - silently ate that world's output from then on, and the
    // repair (ResyncRequired -> gap-fill) was itself re-dropped for being "below _max_seq
    // with no recorded gap". Four rounds of server-side fixes (03ab4f9, bbb8837, 6c846db,
    // 122633b) each closed one poisoning path and left that design intact. Ranges are the
    // complete inverse record: holes are simply the spaces between them, so a hole exists
    // whether or not we ever noticed it opening, and membership is exact.
    //
    // A range is recorded for the WHOLE DELIVERED BATCH SPAN, never per surviving line.
    // Lines this client filters out for display (ANSI-only lines, idler markers, grep mode
    // - see the ServerData handler) still consumed real seqs server-side, so counting only
    // kept lines would punch phantom holes and make us re-request data we already have.
    // This is the same property the old `_max_seq` advance had, and the reason Phase B's
    // rawIdx-based per-line seq derivation is load-bearing.
    //
    // Carried across InitialState (both the in-memory reconnect and IndexedDB cache
    // hydration paths) and persisted alongside cached lines, so a hole known before a
    // reconnect isn't silently forgotten.
    //
    // Bounded by coalescing: the array holds (number of open holes + 1) entries, which is
    // normally exactly 1. The cap below is a backstop for a pathological stream; on
    // overflow the OLDEST hole is declared permanently absent by merging the first two
    // ranges. That only ever moves the frontier forward, so an unrecoverable seq can never
    // stall the resume/ack contract behind it.
    const MAX_SEEN_SEQ_RANGES = 512;

    // Record [start, end] (inclusive) as delivered, coalescing with any adjacent or
    // overlapping ranges so the array stays sorted, minimal, and non-overlapping.
    function markSeqRangeSeen(world, start, end) {
        if (!world || start === undefined || start === null) return;
        if (end === undefined || end === null || end < start) end = start;
        if (!world._seenRanges) world._seenRanges = [];
        const ranges = world._seenRanges;

        // First range that could touch or follow [start, end]. `r.end >= start - 1` rather
        // than `>= start` so an exactly-adjacent range coalesces instead of staying split.
        let i = 0;
        while (i < ranges.length && ranges[i].end < start - 1) i++;

        // Absorb every range that touches or overlaps the new one.
        let newStart = start;
        let newEnd = end;
        let removeCount = 0;
        while (i + removeCount < ranges.length && ranges[i + removeCount].start <= end + 1) {
            const r = ranges[i + removeCount];
            if (r.start < newStart) newStart = r.start;
            if (r.end > newEnd) newEnd = r.end;
            removeCount++;
        }
        ranges.splice(i, removeCount, { start: newStart, end: newEnd });

        // Backstop only - see MAX_SEEN_SEQ_RANGES. Merging ranges[0] into ranges[1] closes
        // the oldest hole by fiat, which advances the frontier rather than stalling it.
        while (ranges.length > MAX_SEEN_SEQ_RANGES) {
            ranges[1].start = ranges[0].start;
            ranges.shift();
        }
    }

    // Exact membership test: has the server already delivered this seq to us?
    function hasSeenSeq(world, seq) {
        if (!world || !world._seenRanges || seq === undefined || seq === null) return false;
        const ranges = world._seenRanges;
        let lo = 0;
        let hi = ranges.length - 1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if (seq < ranges[mid].start) hi = mid - 1;
            else if (seq > ranges[mid].end) lo = mid + 1;
            else return true;
        }
        return false;
    }

    // Whole-span membership: has every seq in [start, end] already been delivered? True
    // exactly when a single coalesced range contains both endpoints (ranges are
    // non-overlapping and non-adjacent by construction, so no other arrangement can cover
    // the span). Kept O(log n) rather than looping the span - a batch's end_seq comes off
    // the wire, and a corrupt or hostile value must not turn this into an unbounded loop.
    function hasSeenRange(world, start, end) {
        if (!world || !world._seenRanges || start === undefined || start === null) return false;
        if (end === undefined || end === null || end < start) end = start;
        const ranges = world._seenRanges;
        let lo = 0;
        let hi = ranges.length - 1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if (start < ranges[mid].start) hi = mid - 1;
            else if (start > ranges[mid].end) lo = mid + 1;
            else return end <= ranges[mid].end;
        }
        return false;
    }

    // The oldest hole in this world's delivered-seq record, as [start, end] inclusive, or
    // null when there is none. Holes are simply the spaces between ranges, so this is
    // ranges[0].end + 1 .. ranges[1].start - 1.
    function oldestSeqHole(world) {
        const r = world && world._seenRanges;
        if (!r || r.length < 2) return null;
        return { start: r[0].end + 1, end: r[1].start - 1 };
    }

    // Declare the oldest hole permanently absent by merging the first two ranges, which
    // advances contiguousFrontier() past it. Same operation as the MAX_SEEN_SEQ_RANGES
    // backstop, invoked deliberately instead of on overflow.
    //
    // This exists because a hole the server cannot fill is otherwise unrecoverable AND
    // self-perpetuating. requestGapFill() anchors on contiguousFrontier(), so an unfillable
    // seq freezes that anchor: every reply then re-delivers only lines we already hold, the
    // frontier doesn't move, and - because the reply is a full chunk, so backfill_complete is
    // false - the ScrollbackLines handler immediately requests the same range again. That is
    // an unbounded request loop on a live socket, on the client most likely to be on a
    // metered connection. A seq can genuinely become unfillable: Ctrl+L's selective flush and
    // a splash clear both remove lines from the server's output_lines, and the archive
    // prepend caps output_lines at 10k lines - after any of those the line behind that seq no
    // longer exists to be re-sent. Giving up on the run is the only terminating outcome; the
    // loss is reported (ReportGap -> SEQ-HOLE) rather than silently absorbed.
    function closeOldestSeqHole(world) {
        const r = world && world._seenRanges;
        if (!r || r.length < 2) return false;
        r[1].start = r[0].start;
        r.shift();
        world._max_seq = maxSeenSeq(world);
        return true;
    }

    // Whether we hold ANY delivered seq for this world. Distinct from `contiguousFrontier()
    // > 0` and from `_max_seq` truthiness: seq 0 is a real, legitimate value (World::next_seq
    // starts at 0), so a world whose only delivered line is seq 0 has genuine data even
    // though both of those read as falsy. Guarding on truthiness made such a world skip the
    // newer-lines gap-fill entirely and fetch only older history.
    function hasDeliveredSeqs(world) {
        return !!(world && world._seenRanges && world._seenRanges.length > 0);
    }

    // --- Resume/ack contract (PROTOCOL-ROADMAP.md Step 5) ----------------------------
    // The reconnect and keepalive-ack contract needs the highest seq such that EVERY seq up
    // to and including it has actually been received - not the highest seq SEEN, which can
    // sit past an unrecovered hole. Telling the server "I have everything up to _max_seq"
    // when we don't permanently hides that hole from the exact resume replay meant to fix
    // it. With ranges this is exact and O(1): everything from the start of what we track
    // through ranges[0].end is contiguous by construction, and ranges[0].end is therefore
    // the boundary. Also feeds the Phase C server-side ack audit, which compares it against
    // the world's true deliverable high seq.
    //
    // Erring LOW here is safe and erring high is not, which is why this takes ranges[0]
    // rather than trying to be clever about a disjoint older range. Too low costs a larger
    // replay, and every re-sent line is now skipped exactly by hasSeenSeq() on arrival. Too
    // high is the permanent-loss bug this whole phase exists to remove.
    function contiguousFrontier(world) {
        if (!world || !world._seenRanges || world._seenRanges.length === 0) return 0;
        return world._seenRanges[0].end;
    }

    // Highest seq the server has delivered for this world (the old `_max_seq`). Kept as a
    // function so the two can't drift; `world._max_seq` is still written in lockstep for
    // the existing readers that only need the high-water value.
    function maxSeenSeq(world) {
        if (!world || !world._seenRanges || world._seenRanges.length === 0) return 0;
        return world._seenRanges[world._seenRanges.length - 1].end;
    }

    // Recompute _seenRanges from a hydrated buffer's real per-line seqs, then union in any
    // ranges carried over from the previous session (in-memory reconnect or IndexedDB
    // cache). The union matters: the buffer is capped (cache) or trimmed (ring), so it can
    // hold fewer seqs than we were actually delivered, and dropping the carried ranges
    // would resurrect holes we'd already filled. Lines tagged _has_real_seq === false carry
    // an array index rather than a seq (the seq: 0 ephemeral-broadcast fallback) and are
    // excluded - see the recompute loops in the InitialState handler for the full rationale.
    function rebuildSeenRanges(world, carriedRanges) {
        world._seenRanges = [];
        for (const r of (carriedRanges || [])) {
            if (r && typeof r.start === 'number' && typeof r.end === 'number') {
                markSeqRangeSeen(world, r.start, r.end);
            }
        }
        for (const line of (world.output_lines || [])) {
            if (line && line._has_real_seq !== false && line.seq !== undefined) {
                markSeqRangeSeen(world, line.seq, line.seq);
            }
        }
        world._max_seq = maxSeenSeq(world);
    }

    // Detect a scrollback buffer poisoned by the pre-1.5.23 archive bug.
    //
    // Servers before 1.5.23 could put archived scrollback (loaded from scrollback.db when the
    // operator scrolled to the top in the TUI) onto the wire as ordinary ServerData. Those
    // lines carry FABRICATED seqs, counted backwards from the buffer's oldest and saturating
    // at 0, so they can overlap the live range. insertLinesBySeq then orders them by seq
    // rather than arrival, which interleaves them into the middle of real history - the
    // reported symptom was a world showing 8/13 text ABOVE 7/02 and 8/5 text, with nothing
    // new ever appearing at the bottom because the frontier had been dragged up to the
    // archive's highest fabricated seq.
    //
    // A fixed server never sends these, so their presence is proof the buffer predates the
    // fix. Out-of-order real seqs are the same damage seen from the other side, and catch a
    // buffer whose archive lines have since been trimmed out of the cache cap.
    //
    // Discarding is the safe direction: the worst case is one extra download of history the
    // server still has, versus a world that stays permanently frozen.
    // Backwards timestamp jump (seconds) treated as proof of a scrambled buffer. Generous on
    // purpose: live output is timestamped on arrival and only ever moves forward, and lines
    // recovered out of order are re-inserted in seq order, so they stay monotonic too. A jump
    // this large mid-buffer means content from a different era was interleaved.
    const CORRUPT_TS_REGRESSION_SECS = 3600;

    function bufferIsCorrupted(lines) {
        if (!Array.isArray(lines) || lines.length === 0) return false;
        let prevSeq = null;
        let prevTs = null;
        for (const line of lines) {
            if (!line) continue;
            // Direct evidence: the server marked this line as archived. Only a pre-1.5.23
            // server ever put one on the wire.
            if (line.from_archive) return true;
            // Indirect evidence, and the only signal available when the lines arrived via the
            // broadcast-ledger repair path - that resent them as ServerData, which carries no
            // per-line from_archive flag at all. This is the reported shape: current text
            // sitting ABOVE text from weeks earlier, because the archived lines were handed
            // seqs above the live range and insertLinesBySeq ordered them last.
            if (typeof line.ts === 'number' && line.ts > 0) {
                if (prevTs !== null && line.ts < prevTs - CORRUPT_TS_REGRESSION_SECS) return true;
                prevTs = Math.max(prevTs === null ? line.ts : prevTs, line.ts);
            }
            // Lines tagged _has_real_seq === false carry an array index, not a seq (the
            // seq: 0 ephemeral-broadcast fallback) - they say nothing about ordering.
            if (line._has_real_seq === false || typeof line.seq !== 'number') continue;
            // A strict inversion, or a repeat of a non-zero seq. Plain seq 0 is excluded
            // from the duplicate test: the OutputLines handler coerces a missing seq to 0
            // (`line.seq || 0`), so two of those in a row are not evidence of damage.
            if (prevSeq !== null && (line.seq < prevSeq || (line.seq === prevSeq && line.seq !== 0))) return true;
            prevSeq = line.seq;
        }
        return false;
    }

    // Convert a legacy cache entry ({maxSeq, seqGaps}) into ranges, so upgrading doesn't
    // discard anyone's persisted scrollback (no IndexedDB version bump - same reasoning as
    // the CLIENT_LINE_PREFIX migration in the InitialState handler). The old record was
    // "everything from 0..maxSeq except these gaps", which maps exactly onto ranges.
    function seenRangesFromLegacyGaps(maxSeq, seqGaps) {
        if (!maxSeq || maxSeq <= 0) return [];
        const gaps = (seqGaps || [])
            .filter(g => g && typeof g.start === 'number' && typeof g.end === 'number')
            .slice()
            .sort((a, b) => a.start - b.start);
        const ranges = [];
        let cursor = 0;
        for (const g of gaps) {
            if (g.start > cursor) ranges.push({ start: cursor, end: Math.min(g.start - 1, maxSeq) });
            cursor = Math.max(cursor, g.end + 1);
        }
        if (cursor <= maxSeq) ranges.push({ start: cursor, end: maxSeq });
        return ranges.filter(r => r.end >= r.start);
    }

    // Builds the (world_index, last_contiguous_seq) list shared by AuthRequest.resume
    // (sent on connect/reconnect) and PongCheck.acked (sent periodically on the
    // keepalive cycle) - same shape, same semantics, see websocket.rs. Only worlds with
    // real received history are included; a world with nothing yet has nothing to
    // resume/ack.
    function buildResumeAckList() {
        const list = [];
        worlds.forEach((world, idx) => {
            const seq = contiguousFrontier(world);
            if (seq > 0) list.push([idx, seq]);
        });
        return list;
    }

    // Splice line objects (already carrying real .seq values) into world.output_lines at
    // the position seq order dictates, rather than assuming they belong at the tail —
    // used only for recovered gap-fill batches, which are older than what's already there.
    function insertLinesBySeq(world, newLineObjs) {
        if (newLineObjs.length === 0) return;
        const firstSeq = newLineObjs[0].seq;
        let insertAt = 0;
        for (let i = world.output_lines.length - 1; i >= 0; i--) {
            const existing = world.output_lines[i];
            if (existing._has_real_seq && existing.seq < firstSeq) {
                insertAt = i + 1;
                break;
            }
        }
        world.output_lines.splice(insertAt, 0, ...newLineObjs);
    }

    // One-time dedup pass for lines that may already have been duplicated by the seq-drift
    // bug Step 8/9 fix (PROTOCOL-ROADMAP.md's seq-drift fix): a client-side seq that had
    // drifted below the server's true value could record a phantom gap, and the server's
    // resume replay would then faithfully re-send lines the client already had, which got
    // appended as duplicates - both into a live reconnect's in-memory buffer and, once
    // persisted via scheduleWorldCacheSave, into the IndexedDB cache too. Applied on hydrate
    // (both the priorWorld and cachedWorld branches below) so a client/cache that already
    // picked up duplicates before this fix shipped gets cleaned up on its next
    // reconnect/cold-start, without needing a cache DB version bump - which would silently
    // discard every user's cached scrollback (see the CLIENT_LINE_PREFIX migration's
    // precedent just below). Deliberately seq-keyed (exact, not a heuristic text match) so it
    // can never misfire on legitimately repeated text (e.g. a MUD prompt repeated verbatim).
    // Lines without a real seq (_has_real_seq === false, or absent - see the recompute
    // loops' `!== false` reasoning) are left untouched; they have no seq to key on and were
    // never a product of the resume-replay duplication this targets.
    //
    // Residual case, not covered here: a duplicate that entered via a `seq: 0` release
    // broadcast (before PROTOCOL-ROADMAP.md's Step 6 made release broadcasts carry real
    // seqs) has no real seq at all and is invisible to this pass. Step 6 means no *new*
    // seq-less duplicates are produced going forward, so this is only a residual risk for
    // buffers/caches that predate that fix; add a bounded heuristic text-repeat pass here
    // only if field reports show it's still needed.
    function dedupBySeq(lines) {
        const seen = new Set();
        const result = [];
        for (const line of lines) {
            if (line && line._has_real_seq !== false && line.seq !== undefined) {
                if (seen.has(line.seq)) continue;
                seen.add(line.seq);
            }
            result.push(line);
        }
        return result;
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
        statusNoteBtn: document.getElementById('status-note-btn'),
        statusScrollback: document.getElementById('status-scrollback'),
        statusScrollbackPct: document.getElementById('status-scrollback-pct'),
        statusTime: document.getElementById('status-time'),
        statusBar: document.getElementById('status-bar'),
        statusItem: document.querySelector('#status-bar .status-item'),
        // World-tabs ribbon
        tabsRibbon: document.getElementById('tabs-ribbon'),
        tabsRibbonTabs: document.getElementById('tabs-ribbon-tabs'),
        tabsRibbonLeft: document.getElementById('tabs-ribbon-left'),
        tabsRibbonRight: document.getElementById('tabs-ribbon-right'),
        // Icon bar
        iconBar: document.getElementById('icon-bar'),
        iconBarDivider: document.getElementById('icon-bar-divider'),
        iconBarLeft: document.getElementById('icon-bar-left'),
        iconBarRight: document.getElementById('icon-bar-right'),
        iconBarShortcuts: document.getElementById('icon-bar-shortcuts'),
        iconBarTagsTile: document.getElementById('icon-bar-tags-tile'),
        worldMenuDropdown: document.getElementById('world-menu-dropdown'),
        // Note editor (NOTE_MODE only — own window/tab, see webview_gui.rs's WvEvent::NoteWindow)
        noteEditorView: document.getElementById('note-editor-view'),
        noteEditorTitle: document.getElementById('note-editor-title'),
        noteEditorStatus: document.getElementById('note-editor-status'),
        noteEditorSaveBtn: document.getElementById('note-editor-save-btn'),
        noteEditorCancelBtn: document.getElementById('note-editor-cancel-btn'),
        noteEditorTextarea: document.getElementById('note-editor-textarea'),
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
        // Connection log modal
        connectionLogRetryBtn: document.getElementById('connection-log-retry-btn'),
        connectionLogCancelBtn: document.getElementById('connection-log-cancel-btn'),
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
        // Menu
        menuBtn: document.getElementById('menu-btn'),
        menuDropdown: document.getElementById('menu-dropdown'),
        // Font slider (status bar)
        fontSliderInput: document.getElementById('font-slider'),
        fontSliderLabel: document.getElementById('font-slider-label'),
        fontSliderVal: document.getElementById('font-slider-val'),
        // Nav bar (tablet/phone)
        navBar: document.getElementById('nav-bar'),
        navMenuBtn: document.getElementById('nav-menu-btn'),
        navPgUpBtn: document.getElementById('nav-pgup-btn'),
        navPgDnBtn: document.getElementById('nav-pgdn-btn'),
        navUpBtn: document.getElementById('nav-up-btn'),
        navDownBtn: document.getElementById('nav-down-btn'),
        navFontSlider: document.getElementById('nav-font-slider'),
        navFontSliderLabel: document.getElementById('nav-font-slider-label'),
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
        actionPatternsContainer: document.getElementById('action-patterns-container'),
        actionAddPatternBtn: document.getElementById('action-add-pattern-btn'),
        actionEditorPageBtn: document.getElementById('action-editor-page-btn'),
        actionCommand: document.getElementById('action-command'),
        actionEnabled: document.getElementById('action-enabled'),
        actionStartup: document.getElementById('action-startup'),
        actionGuiShortcut: document.getElementById('action-gui-shortcut'),
        actionSuppressBlanks: document.getElementById('action-suppress-blanks'),
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
        webPortSelect: document.getElementById('web-port-select'),
        webCustomPortField: document.getElementById('web-custom-port-field'),
        webCustomPort: document.getElementById('web-custom-port'),
        webPath: document.getElementById('web-path'),
        webAllowList: document.getElementById('web-allow-list'),
        webWsPassword: document.getElementById('web-ws-password'),
        webCustomCertSelect: document.getElementById('web-custom-cert-select'),
        webCertFile: document.getElementById('web-cert-file'),
        webKeyFile: document.getElementById('web-key-file'),
        tlsCertField: document.getElementById('tls-cert-field'),
        tlsKeyField: document.getElementById('tls-key-field'),
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
        webModifyKeyBtn: document.getElementById('web-modify-key-btn'),
        // Setup fields (inside combined settings modal)
        setupMoreModeToggle: document.getElementById('setup-more-mode-toggle'),
        setupAnsiMusicToggle: document.getElementById('setup-ansi-music-toggle'),
        setupZwjToggle: document.getElementById('setup-zwj-toggle'),
        setupTtsSelect: document.getElementById('setup-tts-select'),
        setupTtsSpeakModeSelect: document.getElementById('setup-tts-speak-mode-select'),
        setupTabsSelect: document.getElementById('setup-tabs-select'),
        setupIconBarSelect: document.getElementById('setup-iconbar-select'),
        setupTlsProxyToggle: document.getElementById('setup-tls-proxy-toggle'),
        setupNewLineIndicatorToggle: document.getElementById('setup-new-line-indicator-toggle'),
        setupKeyboardVisibleToggle: document.getElementById('setup-keyboard-visible-toggle'),
        setupDebugToggle: document.getElementById('setup-debug-toggle'),
        setupArchiveToggle: document.getElementById('setup-archive-toggle'),
        setupLogInputField: document.getElementById('setup-log-input-field'),
        setupLogInputToggle: document.getElementById('setup-log-input-toggle'),
        setupWorldSwitchSelect: document.getElementById('setup-world-switch-select'),
        setupInputHeightValue: document.getElementById('setup-input-height-value'),
        setupHeightMinus: document.getElementById('setup-height-minus'),
        setupHeightPlus: document.getElementById('setup-height-plus'),
        setupColorOffsetValue: document.getElementById('setup-color-offset-value'),
        setupColorOffsetMinus: document.getElementById('setup-color-offset-minus'),
        setupColorOffsetPlus: document.getElementById('setup-color-offset-plus'),
        setupWrapspaceValue: document.getElementById('setup-wrapspace-value'),
        setupWrapspaceMinus: document.getElementById('setup-wrapspace-minus'),
        setupWrapspacePlus: document.getElementById('setup-wrapspace-plus'),
        setupRemoteLinesInput: document.getElementById('setup-remote-lines-input'),
        setupThemeSelect: document.getElementById('setup-theme-select'),
        setupTransparencyRow: document.getElementById('setup-transparency-row'),
        setupTransparencySlider: document.getElementById('setup-transparency-slider'),
        setupTransparencyValue: document.getElementById('setup-transparency-value'),
        // Filter popup (F4)
        filterPopup: document.getElementById('filter-popup'),
        filterInput: document.getElementById('filter-input'),
        filterCloseBtn: document.getElementById('filter-close-btn'),
        // Search popup (F5)
        searchPopup: document.getElementById('search-popup'),
        searchInput: document.getElementById('search-input'),
        searchMatchInfo: document.getElementById('search-match-info'),
        searchCloseBtn: document.getElementById('search-close-btn'),
        // Help popup (/help)
        helpModal: document.getElementById('help-modal'),
        helpTitle: document.getElementById('help-title'),
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
    let lastGoodPassword = null;  // Last password that succeeded; kept in memory for silent re-auth across reconnects (not persisted to storage)
    let lastGoodUsername = null;  // Matching username for lastGoodPassword (multiuser mode)
    let authKey = null;  // Device auth key for passwordless authentication
    let authKeyPending = false;  // True when trying key-based auth (to fall back to password on failure)
    let keyAuthFailed = false;   // Set after key rejection so reconnect skips key auth and shows password prompt
    let serverChallenge = '';  // Challenge from ServerHello for challenge-response auth
    let worlds = [];
    let currentWorldIndex = 0;
    let versionMismatchShown = false;  // Warn on client/server version drift once per session, not every reconnect

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

    // Note editor mode: own window/tab showing just a note-editing form for
    // one world (set by /note or URL ?note=<world_index>, mirrors grep mode).
    var noteMode = null;
    if (window.NOTE_MODE) {
        noteMode = window.NOTE_MODE;
    } else if (urlParams.get('note') !== null) {
        var noteWorldIndex = parseInt(urlParams.get('note'), 10);
        if (!isNaN(noteWorldIndex)) {
            noteMode = { world_index: noteWorldIndex };
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
    // Two phases: Phase 1 is a fast breadth-first pass giving every world a
    // screenful (current world first) so switching worlds shows content
    // immediately. Phase 2 tops each world up to its per-world total target,
    // round-robin (one chunk per world per cycle) so no single world's deep
    // history blocks the others from filling.
    let backfillInProgress = false;
    let backfillPhase = 1; // 1 = fast screenful pass, 2 = round-robin deep fill
    let backfillWorldQueue = [];
    let backfillCurrentWorld = null;
    let backfillPhase1Target = 75; // recomputed per-connect: max(75, viewport lines)
    let backfillTotalTarget = 100; // recomputed per-connect: max(remoteInitialLines, phase1Target)
    // Chunk size and inter-chunk delay are purely client-side pacing - the daemon
    // does no throttling of its own (RequestScrollback is a cheap in-memory slice).
    // 500 lines comfortably fits under the WebSocket max_frame_size (256 KiB, see
    // websocket.rs) given MAX_LINE_LENGTH, so a bigger chunk trades a few more bytes
    // per message for far fewer round trips. The delay only needs to be nonzero to
    // yield to the event loop between requests, not to pace against the server.
    const BACKFILL_PHASE2_CHUNK_SIZE = 500;
    const BACKFILL_DELAY_MS = 30;

    // --- ScrollbackLines request/reply correlation (PROTOCOL-ROADMAP.md's seq-drift fix,
    // Bug 2: the 90%-stuck scrollback indicator) --------------------------------------
    // Before this, a ScrollbackLines reply was routed purely on world._gapFillPending -
    // ambiguous whenever a gap-fill and an ordinary backfill chunk could both be
    // outstanding for the same world, and permanently wrong after a RequestState-driven
    // resync (see _resumedFromServer below), which left _gapFillPending stuck true with no
    // server reply ever coming to clear it. request_id (websocket.rs) lets the reply name
    // exactly which request it answers. id 0 is reserved for a server-initiated unprompted
    // resume replay (never sent by this client, only received); ids from
    // nextScrollbackRequestId are used for every client-initiated request this client
    // tracks. A reply whose request_id isn't in pendingScrollbackRequests and isn't 0 (an
    // old server, or a request this client didn't itself register) falls back to the
    // legacy world._gapFillPending heuristic.
    let nextScrollbackRequestId = 1;
    const pendingScrollbackRequests = new Map(); // id -> { kind: 'gapfill'|'backfill'|'initial-fill', worldIndex, timer }
    const SCROLLBACK_REQUEST_TIMEOUT_MS = 15000;

    // Registers a new outstanding scrollback request and returns its id. `kind` is
    // 'gapfill' (a reconnect/resync catch-up, handled as an append) or 'backfill' (an
    // older-history request, handled as a prepend). The watchdog fires if no reply ever
    // arrives - covers the two documented silent-no-reply cases server-side:
    // handle_request_scrollback returning nothing for an out-of-range world_index
    // (main.rs), and handle_request_scrollback_owned's deliberate multiuser no-op on an
    // owner mismatch (main.rs) - either of which would otherwise leave the backfill pump
    // stalled forever with no way to recover.
    function registerScrollbackRequest(worldIndex, kind) {
        const id = nextScrollbackRequestId++;
        const timer = setTimeout(function() {
            if (!pendingScrollbackRequests.has(id)) return;
            pendingScrollbackRequests.delete(id);
            console.warn('ScrollbackLines request timed out, advancing pump anyway', { id, worldIndex, kind });
            const world = worlds[worldIndex];
            if (world && kind === 'gapfill') world._gapFillPending = false;
            updateScrollbackProgress();
            backfillNextWorld();
        }, SCROLLBACK_REQUEST_TIMEOUT_MS);
        pendingScrollbackRequests.set(id, { kind: kind, worldIndex: worldIndex, timer: timer });
        return id;
    }

    // Watchdog for the server's UNPROMPTED resume replay (`request_id: 0`). That reply is
    // never registered in pendingScrollbackRequests - the client didn't ask for it, so
    // there's no id to register - which means it has no timeout of its own. Without this,
    // a _gapFillPending set from _resumedFromServer stays true forever if the replay never
    // arrives, and a stuck _gapFillPending excludes the world from BOTH backfill queues
    // (see startBackfill). Belt-and-braces alongside the resumeSentThisConnection lifetime
    // fix in the InitialState handler: that stops the flag being set wrongly, this stops it
    // being stuck if the replay is genuinely lost.
    const unpromptedReplayTimers = {};   // world index -> timer id
    function armUnpromptedReplayWatchdog(worldIndex) {
        if (unpromptedReplayTimers[worldIndex]) clearTimeout(unpromptedReplayTimers[worldIndex]);
        unpromptedReplayTimers[worldIndex] = setTimeout(function() {
            delete unpromptedReplayTimers[worldIndex];
            const world = worlds[worldIndex];
            if (!world || !world._gapFillPending) return;
            console.warn('Unprompted resume replay never arrived; clearing _gapFillPending and catching up', { worldIndex });
            world._gapFillPending = false;
            requestGapFill(worldIndex);
        }, SCROLLBACK_REQUEST_TIMEOUT_MS);
    }
    function clearUnpromptedReplayWatchdog(worldIndex) {
        if (unpromptedReplayTimers[worldIndex]) {
            clearTimeout(unpromptedReplayTimers[worldIndex]);
            delete unpromptedReplayTimers[worldIndex];
        }
    }

    // Clears the bookkeeping for a request once its reply has arrived (or the request is
    // being abandoned, e.g. on flush/world removal) - cancels the watchdog timer so it
    // doesn't fire spuriously after the fact.
    function resolveScrollbackRequest(id) {
        const entry = pendingScrollbackRequests.get(id);
        if (entry) {
            clearTimeout(entry.timer);
            pendingScrollbackRequests.delete(id);
        }
        return entry;
    }

    // Tracks the exact AuthRequest.resume list sent on THIS connection attempt, keyed by
    // world NAME (matched at InitialState time, mirroring priorWorldsByName), so
    // world._resumedFromServer can be derived from "did we actually ask the server to
    // resume this world" rather than a heuristic (priorWorld && contiguousFrontier(priorWorld)
    // > 0). That heuristic stayed true across a RequestState-driven resync (window.
    // triggerResync(), Android's background-wake path) even though RequestState carries no
    // resume list and the server sends no unprompted replay for it - permanently stuck
    // _gapFillPending with nothing ever able to clear it. Cleared on every socket close
    // (handleSessionDisconnect) so a stale entry from a previous connection attempt can't
    // be misread as "sent this connection".
    let resumeSentThisConnection = new Map(); // world name -> { index }

    // Builds the AuthRequest.resume list AND records it into resumeSentThisConnection -
    // used only at AuthRequest send sites. PongCheck.acked reuses buildResumeAckList()
    // directly without recording, since a periodic keepalive ack is not a resume request.
    // Per-world seq_epoch for the same worlds buildResumeAckList() reports a frontier for,
    // sent as AuthRequest.resume_epochs. The server uses it to skip re-sending history we
    // already hold and are about to keep (see App::build_initial_state_with_resume): on an
    // in-memory reconnect the InitialState handler hydrates a resumed world from
    // priorWorld.output_lines and never reads its output_lines_ts, so those lines were
    // being shipped only to be dropped.
    //
    // The `contiguousFrontier(world) > 0` test MUST stay identical to buildResumeAckList's.
    // The server skips history for exactly the worlds named here, and the missing tail is
    // covered by the resume replay, which fires for exactly the worlds named in `resume`.
    // A world in this list but not in that one would be skipped with nothing replayed to
    // cover it - i.e. an empty world. Same predicate, same set, no such world.
    //
    // A world with no recorded epoch is omitted rather than sent as 0: 0 is the server's
    // "no epoch" value, and it must not compare equal to anything.
    // This build's version, sent as AuthRequest.client_version and logged server-side next to
    // WS-AUTH. On Android the APK bundles its own copy of this file, so it can lag arbitrarily
    // far behind the server it talks to - which is exactly the ambiguity this resolves. A web
    // client is served by the server and so always matches it; empty is logged as "-".
    function clientVersion() {
        try {
            if (window.Android && typeof window.Android.getAppVersion === 'function') {
                return window.Android.getAppVersion() || '';
            }
        } catch (e) { /* fall through - reporting a version must never block auth */ }
        return (typeof window.CLAY_VERSION === 'string') ? window.CLAY_VERSION : '';
    }

    function buildResumeEpochList() {
        const list = [];
        worlds.forEach((world, idx) => {
            if (contiguousFrontier(world) > 0 && world._seq_epoch) {
                list.push([idx, world._seq_epoch]);
            }
        });
        return list;
    }

    function buildResumeAckListForAuthRequest() {
        const list = buildResumeAckList();
        resumeSentThisConnection = new Map();
        for (const pair of list) {
            const w = worlds[pair[0]];
            if (w && w.name) resumeSentThisConnection.set(w.name, { index: pair[0] });
        }
        return list;
    }

    // Coalesced repaint for the current world while it's at the bottom during
    // backfill (see the ScrollbackLines handler). A fast backfill can deliver many
    // chunks within a few hundred ms; debouncing collapses a burst into a single
    // renderOutput() once it quiets down instead of rebuilding the DOM per chunk.
    //
    // The MAX_WAIT ceiling is what makes that safe. Without it this is a pure trailing
    // edge, and a reply stream arriving faster than the debounce window resets the timer
    // on every chunk, so the current world repaints ZERO times until the stream stops.
    // The backfill pump re-arms every BACKFILL_DELAY_MS (30ms) and the gap-fill loop
    // re-requests synchronously, so that is the normal case, not a pathological one: the
    // lines sat in world.output_lines while the DOM stayed minutes-old-looking, and the
    // screen only snapped up to date once catch-up finished. Staleness must be bounded by
    // construction rather than by however long the server takes to finish sending.
    const CURRENT_WORLD_REPAINT_DEBOUNCE_MS = 120;
    const CURRENT_WORLD_REPAINT_MAX_WAIT_MS = 300;
    let currentWorldRepaintTimer = null;
    let currentWorldRepaintBurstStart = 0;   // 0 = no burst in progress
    function runCurrentWorldRepaint() {
        currentWorldRepaintTimer = null;
        currentWorldRepaintBurstStart = 0;   // next call starts a fresh ceiling window
        requestAnimationFrame(renderOutput);
    }
    function scheduleCurrentWorldRepaint() {
        const now = Date.now();
        if (currentWorldRepaintBurstStart === 0) {
            currentWorldRepaintBurstStart = now;
        } else if (now - currentWorldRepaintBurstStart >= CURRENT_WORLD_REPAINT_MAX_WAIT_MS) {
            // Deferred long enough - paint now instead of extending the window again.
            if (currentWorldRepaintTimer !== null) clearTimeout(currentWorldRepaintTimer);
            runCurrentWorldRepaint();
            return;
        }
        if (currentWorldRepaintTimer !== null) clearTimeout(currentWorldRepaintTimer);
        currentWorldRepaintTimer = setTimeout(runCurrentWorldRepaint, CURRENT_WORLD_REPAINT_DEBOUNCE_MS);
    }

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

    // Remote-WebView /connect confirm state - mirrors App::request_remote_attach's
    // pending_remote_connect (main.rs): a second "/connect <same addr>" within 15s
    // confirms the relaunch. Only used by the WEBVIEW_MODE && !AUTO_PASSWORD intercept.
    let pendingRemoteConnect = null; // { addr, requestedAt } or null
    const REMOTE_CONNECT_CONFIRM_WINDOW_MS = 15000;

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

    // Wrapspace: hanging indent (in spaces) for wrapped output continuation rows (0 = off)
    let wrapspace = 0;

    // Remote Lines: lines of scrollback sent to this/other remote clients per world
    // on initial connect (server-side setting, applies to future connects)
    let remoteInitialLines = 100;

    // Report this client's visibility to the server (WsMessage::ClientVisibility). Guarded
    // on an open, authenticated socket: before auth the server has no ClientViewState for us
    // to act on, and the InitialState that follows reports the correct state anyway.
    // Forward any buffered Android lifecycle events to the server, where they land in
    // ~/.clay/remote.log as CLIENT-LIFECYCLE (WsMessage::ReportClientLifecycle).
    //
    // This is how "did the app resume or rebuild itself?" gets answered on a phone that is
    // never going to be plugged into adb: the evidence arrives in the *desktop's* log. Java
    // buffers the events because the most telling one (onCreate) happens long before there is
    // a socket to send it on; draining here means each is reported exactly once.
    function flushAndroidLifecycleEvents() {
        try {
            if (!window.Android || typeof window.Android.takeLifecycleEvents !== 'function') return;
            const blob = window.Android.takeLifecycleEvents();
            if (!blob) return;
            for (const line of String(blob).split('\n')) {
                if (!line) continue;
                const tab = line.indexOf('\t');
                send({
                    type: 'ReportClientLifecycle',
                    event: tab < 0 ? line : line.slice(0, tab),
                    detail: tab < 0 ? '' : line.slice(tab + 1),
                    source: 'android'
                });
            }
        } catch (e) { /* diagnostics must never break the session */ }
    }

    // Drop the Android foreground service. Only call this when the session is genuinely over
    // (the user disconnected, or reconnection has been abandoned) - never on a transient
    // failure. The service is what stops Android reclaiming the process, so killing it during a
    // retry loop is precisely backwards: the app most needs to survive while it is reconnecting.
    function stopAndroidBackgroundService() {
        try {
            if (window.Android && window.Android.stopBackgroundService) {
                window.Android.stopBackgroundService();
            }
        } catch (e) { /* non-fatal */ }
    }

    // Mirrors the server's own ClientViewState.visible so we can tell a real transition from
    // a repeat. handle_client_visibility early-returns on a repeat and sends no ClaimedNew,
    // so the optimistic claim below must fire only when the server will actually act.
    let lastSentVisibility = null;

    // When we last went to the background, so a resume can report how long it was away.
    // Null until the first backgrounding, which reads as awayMs=-1 rather than a bogus age.
    let lastHiddenAt = null;

    // Record a client-side lifecycle event into Android's buffer, so it reaches the server's
    // remote.log as CLIENT-LIFECYCLE on the next flush (see MainActivity.recordClientEvent).
    // Buffered rather than sent directly because the events worth recording happen exactly
    // when there is no usable socket to send them on. A no-op off Android, and on an older
    // Android build whose bridge lacks the method.
    function recordClientEvent(event, detail) {
        try {
            if (window.Android && typeof window.Android.recordClientEvent === 'function') {
                window.Android.recordClientEvent(event, detail);
            }
        } catch (e) { /* diagnostics must never break the path they are reporting on */ }
    }

    function sendClientVisibility(visible) {
        // Stamped before the socket check on purpose: the case worth measuring is a resume
        // that found no usable socket, and if this only ran when one was open, awayMs would
        // be missing from exactly those reports.
        if (!visible) lastHiddenAt = Date.now();
        try {
            if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
                const next = !!visible;
                const transition = lastSentVisibility !== next;
                ws.send(JSON.stringify({ type: 'ClientVisibility', visible: next }));
                lastSentVisibility = next;
                // Android drives this from onPause/onResume (see MainActivity.notifyVisibility),
                // so it is the natural moment to ship any lifecycle events recorded since the
                // last flush - no separate timer or bridge call needed.
                flushAndroidLifecycleEvents();
                // Coming back to the foreground re-claims whatever arrived while we were away
                // (backgrounding drops our markers but leaves the lines viewed, so only truly
                // new text is in play). Claim it before the resume repaint for the same reason
                // as a world switch: the ClaimedNew is a round-trip away.
                if (next && transition) {
                    claimUnviewedLocally(currentWorldIndex);
                    renderOutput();
                }
            }
        } catch (e) { /* non-fatal: the next InitialState re-establishes our state */ }
    }

    // --- ▶ new-text ownership (PROTOCOL-ROADMAP.md, per-line display_id model) -----------
    // How long an unanswered optimistic claim stays revocable. Far longer than any round
    // trip, far shorter than the gap to an unrelated later ClaimedNew.
    const OPTIMISTIC_CLAIM_TTL_MS = 5000;

    // Our server-assigned ownership id, from InitialState.your_display_id. A line renders ▶
    // iff its display_id equals this. 0/null means "we have no id" (an older server), in
    // which case we simply never paint ▶ rather than adopting somebody else's markers.
    let myDisplayId = 0;

    // Stable, client-generated identity that survives reconnects. The server derives our
    // ownership id from it, so a brief transport drop - which allocates a fresh connection
    // id server-side - still resolves to the same id and keeps our markers intact. Persisted
    // in localStorage so it also survives a page reload; falls back to an in-memory value if
    // storage is unavailable (private browsing), which just means markers reset on reload.
    const clientUid = (function() {
        const KEY = 'clay-client-uid';
        try {
            let v = localStorage.getItem(KEY);
            if (!v) {
                v = 'c-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 10);
                localStorage.setItem(KEY, v);
            }
            return v;
        } catch (e) {
            return 'c-mem-' + Math.random().toString(36).slice(2, 10);
        }
    })();

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

    // /import dialog state (see the worldEditorPopupOpen guard below for why this is needed)
    let importDialogOpen = false;
    let importInsecureDialogOpen = false;

    // Web settings popup state (global state from server)
    let settingsPopupOpen = false;
    let settingsActiveTab = 'general';
    // web_secure is no longer user-facing (the server is always TLS-capable for
    // remote clients; localhost is always plain — see http::route_connection).
    // Still synced from/to the server for wire compat with GlobalSettingsMsg.
    let httpEnabled = false;
    let httpPort = 9000;
    let webPath = 'clay';
    let wsEnabled = false;
    let wsPort = 9001;
    let wsAllowList = '';
    let wsCertFile = '';
    let wsKeyFile = '';
    let wsPassword = '';
    let tlsConfigured = false;  // True if server has a custom (user-provided) TLS cert+key configured
    let serverAuthKey = '';  // Auth key from server (for display in web settings)
    // Guards against pushing a full UpdateGlobalSettings snapshot before this client
    // has received the server's real values (InitialState / GlobalSettingsUpdated).
    // Without this, any global still at its JS default (false/'') would overwrite
    // and persist over the server's real setting. See settings-audit investigation.
    let settingsSynced = false;
    // Temporary editing state for web popup (only saved on Save button)
    let editPortMode = 'disabled';  // 'disabled' | '9000' | 'custom'
    let editCustomCert = false;
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
    let setupTabsMode = 'none';
    let setupIconBarMode = 'app_tablet';
    let setupTlsProxy = false;
    let setupNewLineIndicator = false;
    let setupKeyboardAlwaysVisible = true;
    let setupArchive = false;
    let setupLogInput = false;
    let setupDebug = false;
    let setupInputHeightValue = 1;
    let setupWrapspace = 0;
    let setupGuiTheme = 'dark';
    let setupTransparency = 1.0;

    // Filter popup state (F4)
    let filterPopupOpen = false;
    let filterText = '';

    // Search popup state (F5)
    let searchPopupOpen = false;
    let searchText = '';
    let searchMatchIndices = [];  // indices into output_lines that match
    let searchCurrentPos = -1;    // which match is currently shown at bottom

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

    // World-tabs ribbon mode, synced from server settings ('none', 'top', 'bottom')
    let tabsMode = 'none';
    // Icon bar visibility mode, synced from server settings ('none', 'app_tablet', 'all')
    let iconBarMode = 'app_tablet';
    // World-switch dropdown (opened by clicking the world name on the status bar)
    let worldMenuOpen = false;

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
    let keyboardAlwaysVisible = true;  // Will be synced from server settings
    let hardwareKeyboardPresent = false;  // Set by Java via window.onHardwareKeyboardChanged

    // MCMP (MUD Client Media Protocol) state
    let mcmpDefaultUrl = '';
    let mcmpMusicPlayer = null;    // { audio, key, name } - one music track at a time
    let mcmpSoundPlayers = {};     // key -> { audio, name }
    let mcmpMusicFadeTimer = null;

    let tlsProxyEnabled = false;  // TLS proxy for connection preservation over hot reload
    let tempConvertEnabled = false;  // Temperature conversion (32F -> 32F(0C))
    let mouseEnabled = true;  // Console mouse support
    let debugEnabled = false;  // Debug logging
    let scrollbackEnabled = false;  // Long-term archive output
    let logInputEnabled = false;  // Write captured user input to the per-world log file too
    let dictionaryPath = '';  // Custom dictionary path
    let spellCheckEnabled = true;  // Spell checking
    // Track whether the last edit was a deletion - mirrors Rust's
    // App.last_input_was_delete (main.rs), which check_temp_conversion() uses
    // to skip re-converting right after the user undoes a conversion.
    let lastInputWasDelete = false;
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
        if (!window.WEBVIEW_MODE) return;
        sendIpc('opacity:' + alpha);
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

    // Apply wrapspace (hanging indent for wrapped output continuation rows) via the
    // --wrapspace CSS custom property read by #output .line in style.css. No re-render
    // needed — this is pure CSS reflow, so already-visible wrapped lines re-indent
    // instantly the moment the property changes.
    function applyWrapspace(value) {
        document.documentElement.style.setProperty('--wrapspace', String(value));
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
    // This list is verified against parse_command()'s own source by
    // test_command_parity_js_vs_rust in tests.rs
    const INTERNAL_COMMANDS = [
        'help', 'version', 'quit', 'reload', 'update', 'setup', 'web', 'actions',
        'worlds', 'world', 'connections', 'l', 'disconnect', 'dc', 'connect', 'import',
        'flush', 'menu', 'send', 'remote', 'ban', 'unban',
        'testmusic', 'dump', 'notify', 'addworld', 'note', 'tag', 'tags',
        'dict', 'urban', 'translate', 'tr', 'font', 'window', 'url', 'say',
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
        if (!input || input.length === 0) return;

        // Don't convert when the last edit was a deletion - allows undoing a conversion
        if (lastInputWasDelete) return;

        // Only check when cursor is at the end
        if (elements.input.selectionStart !== input.length) return;

        const lastChar = input[input.length - 1];
        // Only trigger on separator characters
        if (!/[\s.,!?;:\)\]\}]/.test(lastChar)) {
            // Non-separator typed - clear skip so next temperature can convert
            skipTempConversion = null;
            return;
        }

        // Pattern: optional minus, then digits with at most one decimal point placed
        // either after some digits ("32", "32.5", "32.") or before a required digit
        // (".5"), optional °, F or C. Mirrors main.rs::check_temp_conversion's
        // backward character scan (requires at least one digit somewhere, but
        // allows a bare leading decimal point) - a plain `\d+\.?\d*` regex would
        // silently drop a leading "." with no digit before it, misreading ".5F" as
        // "5F" (a 10x magnitude error) instead of skipping straight to the "5".
        const match = input.slice(0, -1).match(/(-?(?:\d+\.?\d*|\.\d+))(°?[FfCc])$/);
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

    // Whether the web client should be actively forcing the on-screen keyboard
    // visible right now: the user setting is on, we're on a phone/tablet layout,
    // and no hardware keyboard is attached (hardwareKeyboardPresent is reported
    // by Java via window.onHardwareKeyboardChanged; always false on non-Android
    // clients, so this reduces to the old deviceMode check there).
    function keyboardForceEnabled() {
        return keyboardAlwaysVisible
            && (deviceMode === 'phone' || deviceMode === 'tablet')
            && !hardwareKeyboardPresent;
    }

    // Push the current keyboardForceEnabled() verdict to the native Android layer
    // so it can apply/release windowSoftInputMode and show/hide the IME to match.
    // No-op on non-Android clients (window.Android is undefined there).
    function applyKeyboardForceState() {
        if (!window.Android || typeof window.Android.setKeyboardForceActive !== 'function') return;
        window.Android.setKeyboardForceActive(keyboardForceEnabled());
    }

    // Called by native Android code (MainActivity.onConfigurationChanged) whenever
    // a hardware keyboard is attached or detached, so the force-visible decision
    // stays correct without waiting for the next settings sync.
    window.onHardwareKeyboardChanged = function(present) {
        hardwareKeyboardPresent = !!present;
        applyKeyboardForceState();
    };

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
        applyKeyboardForceState();
        // Device mode affects iconBarVisible() ('app_tablet' mode) - re-render
        // on every device-mode change (auto-detect at startup, or the manual
        // long-press override) so the icon bar's visibility stays correct.
        renderIconBar();
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
        // Kick off the persistent scrollback cache read as early as possible so it
        // has time to finish before InitialState arrives (see the "Reconnect
        // gap-fill and bounded scrollback cache" section below) - this is a local
        // IndexedDB read, normally much faster than the network round trip to
        // connect and authenticate.
        preloadWorldCacheForServer(getServerIdentity());
        // On Android, never auto-connect from init() — Java calls connect() via
        // evaluateJavascript in onPageFinished() after verifying settings exist.
        if (!window.Android) {
            connect();
        }
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

    // Height of one output row in CSS pixels: font-size * the 1.2 line-height in style.css.
    // Single definition because three separate things now convert between pixels and rows -
    // the visible-line count, the "History NNN" distance-from-bottom, and the drag-to-release
    // gesture - and a disagreement between them shows up as the wrong number of lines.
    function lineHeightPx() {
        return (currentFontSize || 14) * 1.2;
    }

    // Get visible line count in output area
    function getVisibleLineCount() {
        return Math.floor(elements.outputContainer.clientHeight / lineHeightPx());
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
    let wakeStateCleared = false;  // True when world connected states were cleared before a wake Ping

    // Parallel connection attempt tracking
    let pendingAttempts = new Map();  // id → { url, proto, isNative, socket, timeout }
    let winnerAttemptId = null;       // id of the winning attempt (null = no winner yet)
    let nextAttemptId = 0;            // monotonic counter for attempt ids
    // Prevent two concurrent connect() calls (visibilitychange + checkConnectionOnResume race)
    let connectInProgress = false;
    let lastForceReconnectAt = 0; // debounce guard against double-trigger on resume
    let forceReconnectRetryTimer = null; // scheduled retry when a call lands inside the debounce window (see forceReconnect)

    // Debug logging - console only (no Toast)
    function debugLog(msg) {
        console.log('[Clay Debug] ' + msg);
    }

    // Check if native Android WebSocket is available (checks capability, not current state)
    function hasNativeWebSocket() {
        try {
            if (!window.Android) return false;
            // Use the Java bridge method (always returns true on Android)
            if (typeof window.Android.hasNativeWebSocket === 'function') {
                return window.Android.hasNativeWebSocket();
            }
            // Fallback: check capability
            return !!(window.Android.connectWebSocket);
        } catch (e) {
            return false;
        }
    }

    // A page served by Clay has {{WEB_PATH}} substituted. A page loaded from bundled
    // APK assets does not — treat the raw placeholder as "not provided" so the Android
    // probe path below engages.
    function injectedWebPath() {
        const v = window.WEB_PATH;
        if (typeof v !== 'string' || v.indexOf('{{') !== -1) return undefined;
        return v;
    }

    // Stealth web-path prefix ("" = legacy mode, server UI lives at "/"). Prefer the
    // value the server injected into this page's template; if unavailable (e.g. the
    // Android WebView loading bundled assets from file:///android_asset, which never
    // goes through the server's template substitution), derive it from the current
    // page path as a best-effort fallback.
    function basePath() {
        const injected = injectedWebPath();
        if (injected !== undefined) {
            return injected ? '/' + injected : '';
        }
        const path = window.location.pathname || '/';
        const segments = path.split('/').filter(Boolean);
        if (segments.length === 0) return '';
        // Mirrors http.rs's KNOWN_ASSET_PATHS (source of truth for the legacy/stealth
        // path split) - keep in sync or a client using this to derive its own asset
        // base path can mis-derive the stealth prefix for a path it doesn't recognize.
        const knownLegacyRoots = ['index.html', 'style.css', 'app.js', 'theme-editor',
            'keybind-editor', 'action-editor', 'favicon.ico', 'clay2.png', 'fonts'];
        return knownLegacyRoots.indexOf(segments[0]) === -1 ? '/' + segments[0] : '';
    }

    // WS upgrade path candidates to try, in order. Browser clients (server-rendered
    // page, window.WEB_PATH known) use exactly one deterministic path. The Android
    // WebView (bundled asset page, no injected WEB_PATH) doesn't know the server's
    // configured web_path, so it tries the new stealth default then the legacy path —
    // safe, since a wrong-path WS upgrade is silently dropped server-side with no
    // ban-violation strike.
    function wsPathCandidates() {
        if (window.Android && injectedWebPath() === undefined) {
            return ['/clay/ws', '/ws'];
        }
        return [basePath() + '/ws'];
    }

    // Build list of WebSocket candidates for this connect cycle.
    function buildCandidates() {
        const local = window.WS_LOCAL_HOST || window.WS_HOST || window.location.hostname;
        const remote = window.WS_REMOTE_HOST || '';
        const port = (window.WS_PORT && window.WS_PORT !== 0)
            ? window.WS_PORT
            : (window.location.port || '443');
        let protos;
        if (window.CONNECTION_MODE) {
            const mode = window.CONNECTION_MODE;
            protos = mode === 'secure' ? ['wss']
                   : mode === 'non_secure' ? ['ws']
                   : ['wss', 'ws'];
        } else {
            protos = (window.WS_PROTOCOL === 'ws') ? ['ws'] : ['wss', 'ws'];
        }
        const hosts = (remote && remote !== local) ? [local, remote] : [local];
        const paths = wsPathCandidates();
        const result = [];
        hosts.forEach(function(h) {
            protos.forEach(function(p) {
                paths.forEach(function(wsPath) {
                    result.push({ proto: p, host: h, url: p + '://' + h + ':' + port + wsPath });
                });
            });
        });
        return result;
    }

    // Post-open logic shared by native and browser WebSocket winners.
    function handleSocketOpen() {
        if (connectionTimeout) { clearTimeout(connectionTimeout); connectionTimeout = null; }
        connectionFailures = 0;
        hideCertWarning();
        setTimeout(hideConnectionLog, 800);

        if (window.AUTO_PASSWORD) {
            ws.send(JSON.stringify({ type: 'AuthRequest', password_hash: window.AUTO_PASSWORD, request_key: false, resume: buildResumeAckListForAuthRequest(), resume_epochs: buildResumeEpochList(), client_version: clientVersion(), client_uid: clientUid }));
            return;
        }

        let savedPassword = null;
        let savedUsername = null;
        try {
            if (window.Android && typeof window.Android.getSavedPassword === 'function') {
                savedPassword = window.Android.getSavedPassword();
                if (typeof savedPassword !== 'string' || savedPassword.trim() === '') savedPassword = null;
            }
            if (window.Android && typeof window.Android.getSavedUsername === 'function') {
                savedUsername = window.Android.getSavedUsername();
                if (typeof savedUsername !== 'string' || savedUsername.trim() === '') savedUsername = null;
            }
        } catch (e) {
            console.error('Error getting saved credentials:', e);
            savedPassword = null;
            savedUsername = null;
        }

        if (savedPassword) {
            if (savedUsername) {
                enableMultiuserAuthUI();
                if (elements.authUsername) elements.authUsername.value = savedUsername;
                authenticate(savedPassword, savedUsername);
            } else {
                deferredAutoLoginPassword = savedPassword;
                deferredAutoLoginUsername = null;
                setTimeout(function() {
                    if (deferredAutoLoginPassword) {
                        const pwd = deferredAutoLoginPassword;
                        deferredAutoLoginPassword = null;
                        authenticate(pwd, null);
                    }
                }, 1000);
            }
        } else if (lastGoodPassword) {
            // Browser silent re-auth after disconnect/hot-reload.
            // Queue password for the ServerHello handler (need the fresh challenge first).
            deferredAutoLoginPassword = lastGoodPassword;
            deferredAutoLoginUsername = lastGoodUsername;
            keyAuthFailed = true;  // Skip key-auth path; go straight to password re-auth
        } else if (authKey && !keyAuthFailed) {
            debugLog('handleSocketOpen: waiting for ServerHello to try key auth');
            setTimeout(function() {
                if (!authenticated && !authKeyPending) {
                    debugLog('ServerHello timeout, showing auth modal');
                    showAuthModal(true);
                    elements.authPassword.focus();
                }
            }, 3000);
        } else {
            showAuthModal(true);
            if (savedUsername && elements.authUsername) {
                enableMultiuserAuthUI();
                elements.authUsername.value = savedUsername;
                elements.authPassword.focus();
            } else {
                elements.authPassword.focus();
            }
        }
    }

    // Called when an attempt's onopen fires — claims winner or closes late loser.
    function handleAttemptWin(id) {
        if (winnerAttemptId !== null) {
            const attempt = pendingAttempts.get(id);
            if (attempt) {
                if (attempt.timeout) clearTimeout(attempt.timeout);
                if (attempt.isNative) {
                    if (window.Android) try { window.Android.closeWebSocket(id); } catch(e) {}
                } else if (attempt.socket) {
                    attempt.socket.onclose = null; attempt.socket.onerror = null;
                    attempt.socket.onopen = null; attempt.socket.onmessage = null;
                    try { attempt.socket.close(); } catch(e) {}
                }
                pendingAttempts.delete(id);
                resolveAttempt(id, false, '(lost)');
            }
            return;
        }

        winnerAttemptId = id;
        connectInProgress = false;

        const attempt = pendingAttempts.get(id);
        if (attempt && attempt.timeout) clearTimeout(attempt.timeout);

        if (attempt && attempt.isNative) {
            const winId = id;
            ws = {
                readyState: WebSocket.OPEN,
                send: function(data) {
                    if (window.Android) window.Android.sendWebSocketMessage(winId, data);
                },
                close: function() {
                    if (window.Android) try { window.Android.closeWebSocket(winId); } catch(e) {}
                    this.readyState = WebSocket.CLOSED;
                }
            };
        } else if (attempt && attempt.socket) {
            ws = attempt.socket;
        }

        pendingAttempts.forEach(function(a, aid) {
            if (aid === id) return;
            if (a.timeout) clearTimeout(a.timeout);
            if (!a.isNative && a.socket) {
                a.socket.onclose = null; a.socket.onerror = null;
                a.socket.onopen = null; a.socket.onmessage = null;
                try { a.socket.close(); } catch(e) {}
            }
            resolveAttempt(aid, false, '(canceled)');
        });
        pendingAttempts.clear();

        if (window.Android && typeof window.Android.closeOtherWebSockets === 'function') {
            try { window.Android.closeOtherWebSockets(id); } catch(e) {}
        }

        resolveAttempt(id, true, '');
        setTimeout(hideConnectionLog, 800);
        handleSocketOpen();
    }

    // Called when a pending attempt fails (before winning the race).
    function handleAttemptFailure(id) {
        const attempt = pendingAttempts.get(id);
        if (!attempt) return;
        if (attempt.timeout) clearTimeout(attempt.timeout);
        pendingAttempts.delete(id);
        resolveAttempt(id, false, '');

        if (pendingAttempts.size === 0 && winnerAttemptId === null) {
            connectInProgress = false;
            // A stray/leftover cycle can lose its race after a different cycle already
            // won and authenticated - don't let it pop the failure dialog back up over
            // a connection that's actually fine (that's the "stuck dialog despite a
            // working connection" bug).
            if (authenticated && ws && ws.readyState === WebSocket.OPEN) {
                debugLog('handleAttemptFailure: already connected+authenticated elsewhere, suppressing');
                return;
            }
            connectionFailures++;
            const maxFailures = window.WEBVIEW_MODE ? 5 : 2;
            if (connectionFailures >= maxFailures) {
                // Only now, having given up, drop the Android foreground service. Tearing it
                // down on every individual failure removed the process's protection at exactly
                // the moment it was mid-reconnect and needed it most - the service is what keeps
                // Android from reclaiming us while we retry.
                stopAndroidBackgroundService();
                showConnectionLog();
                enableConnectionLogRetry();
            } else {
                setTimeout(connect, 2000);
            }
        }
    }

    // Called when the winner socket closes (session disconnect after auth).
    function handleSessionDisconnect(code, reason) {
        debugLog('Session disconnect: ' + code + ' ' + reason);
        if (wakePongTimeout) { clearTimeout(wakePongTimeout); wakePongTimeout = null; }
        if (ws && !(ws instanceof WebSocket)) ws.readyState = WebSocket.CLOSED;
        authenticated = false;
        winnerAttemptId = null;
        // A stale entry from a previous connection attempt must never be misread as
        // "sent this connection" by the next InitialState's _resumedFromServer derivation.
        resumeSentThisConnection = new Map();
        // Any outstanding ScrollbackLines requests belong to the dead connection and will
        // never get a reply now - clear them rather than letting their watchdogs fire late
        // against whatever new state exists after reconnecting.
        for (const id of Array.from(pendingScrollbackRequests.keys())) {
            resolveScrollbackRequest(id);
        }

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

        connectionFailures++;

        // If there is no usable credential and the auth modal is already open,
        // stop auto-reconnecting. The user will trigger a fresh connection by
        // submitting the form (authenticate() calls forceReconnect() when WS is closed).
        // This prevents the 30 s idle-drop flash loop when the user sits at the prompt.
        const canSilentReauth = !!(window.AUTO_PASSWORD || lastGoodPassword ||
            deferredAutoLoginPassword || (authKey && !keyAuthFailed));
        const modalVisible = elements.authModal &&
            elements.authModal.classList.contains('visible');
        if (!canSilentReauth && modalVisible) {
            debugLog('Session disconnect with auth modal open and no credential — not auto-reconnecting');
            return;
        }

        const maxFailures = window.WEBVIEW_MODE ? 5 : 2;
        if (connectionFailures >= maxFailures) {
            // Reconnection abandoned - only now is it right to drop the foreground service
            // (see stopAndroidBackgroundService). While still retrying above, it stays up.
            stopAndroidBackgroundService();
            showConnectionLog();
            enableConnectionLogRetry();
        } else {
            setTimeout(connect, 2000);
        }
    }

    // Set up native WebSocket callbacks (id-scoped for parallel racing)
    function setupNativeWebSocketCallbacks() {
        window.onNativeWebSocketOpen = function(id) {
            debugLog('Native WS OPEN [' + id + ']');
            handleAttemptWin(id);
        };

        window.onNativeWebSocketMessage = function(id, data) {
            if (id !== winnerAttemptId) return;
            let msg;
            try {
                msg = JSON.parse(data);
            } catch (e) {
                console.error('Failed to parse message:', e);
                return;
            }
            // This is the path Android actually uses (NativeWebSocket.java relays into it), so
            // a handler throw here used to vanish into console.error on the one platform with
            // no console. Reported separately from a parse failure - see the browser-socket
            // onmessage below for the same split.
            try {
                handleMessage(msg);
            } catch (e) {
                __clayShowError('handleMessage(' + (msg && msg.type ? msg.type : '?') +
                    ') threw: ' + __clayErrText(e));
            }
        };

        window.onNativeWebSocketMessageBase64 = function(id, base64Data) {
            if (id !== winnerAttemptId) return;
            let msg;
            try {
                const data = atob(base64Data);
                const bytes = new Uint8Array(data.length);
                for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i);
                const decoded = new TextDecoder('utf-8').decode(bytes);
                msg = JSON.parse(decoded);
            } catch (e) {
                console.error('Failed to parse Base64 message:', e);
                return;
            }
            try {
                handleMessage(msg);
            } catch (e) {
                __clayShowError('handleMessage(' + (msg && msg.type ? msg.type : '?') +
                    ') threw: ' + __clayErrText(e));
            }
        };

        // PROTOCOL-ROADMAP.md Step 7: pull side of the ordered-queue Android bridge. Android's
        // MainActivity.java no longer pushes each WebSocket frame through evaluateJavascript()
        // (that was fire-and-forget and could execute out of order under load, even though the
        // underlying OkHttp WebSocket delivers frames in order) - it now just calls this as a
        // content-free "go pull" signal once per enqueued message, and we synchronously drain
        // window.Android.drainWsQueue(), a direct @JavascriptInterface method call (no
        // WebView-internal async dispatch, so no reordering risk) that atomically returns every
        // message queued since the last drain as a JSON array of [id, message] pairs, oldest
        // first. Processed through the exact same handleMessage() path as every other transport,
        // in order, synchronously - ordering is now structurally guaranteed rather than
        // compensated for after the fact. No base64 involved: drainWsQueue() returns raw JSON
        // text directly, since evaluateJavascript is no longer used to carry the payload.
        window.onNativeWsQueueReady = function() {
            if (!window.Android || typeof window.Android.drainWsQueue !== 'function') return;
            let batch;
            try {
                batch = JSON.parse(window.Android.drainWsQueue());
            } catch (e) {
                console.error('Failed to parse native WS queue batch:', e);
                return;
            }
            for (const pair of batch) {
                const id = pair[0];
                const data = pair[1];
                if (id !== winnerAttemptId) continue;
                try {
                    handleMessage(JSON.parse(data));
                } catch (e) {
                    console.error('Failed to parse native WS queue message:', e);
                }
            }
        };

        window.onNativeWebSocketClose = function(id, code, reason) {
            debugLog('Native WS CLOSE [' + id + ']: ' + code + ' ' + reason);
            if (id === winnerAttemptId) {
                handleSessionDisconnect(code, reason);
            } else if (pendingAttempts.has(id)) {
                handleAttemptFailure(id);
            }
        };

        window.onNativeWebSocketError = function(id, error) {
            debugLog('Native WS ERROR [' + id + ']: ' + error);
            if (id === winnerAttemptId) {
                handleSessionDisconnect(1006, error);
            } else if (pendingAttempts.has(id)) {
                handleAttemptFailure(id);
            }
        };
    }

    // Initialize native WebSocket callbacks
    if (hasNativeWebSocket()) {
        setupNativeWebSocketCallbacks();

        window.addEventListener('beforeunload', function() {
            if (!window.Android) return;
            if (winnerAttemptId !== null) {
                try { window.Android.closeWebSocket(winnerAttemptId); } catch(e) {}
            }
            pendingAttempts.forEach(function(a, id) {
                if (a.isNative) try { window.Android.closeWebSocket(id); } catch(e) {}
            });
        });

        window.addEventListener('pagehide', function() {
            if (!window.Android) return;
            if (winnerAttemptId !== null) {
                try { window.Android.closeWebSocket(winnerAttemptId); } catch(e) {}
            }
            pendingAttempts.forEach(function(a, id) {
                if (a.isNative) try { window.Android.closeWebSocket(id); } catch(e) {}
            });
        });
    }

    function connectWithNativeWebSocket(id, url) {
        debugLog('Native WS connecting [' + id + ']: ' + url);
        const timeout = setTimeout(function() {
            if (pendingAttempts.has(id)) {
                console.log('Native WebSocket [' + id + '] timeout');
                if (window.Android) try { window.Android.closeWebSocket(id); } catch(e) {}
                handleAttemptFailure(id);
            }
        }, 5000);
        pendingAttempts.set(id, { url: url, proto: 'wss', isNative: true, timeout: timeout });
        try {
            window.Android.connectWebSocket(id, url);
        } catch (e) {
            console.error('connectWebSocket error:', e);
            handleAttemptFailure(id);
        }
    }

    function connectWithBrowserWebSocket(id, url) {
        debugLog('Browser WS connecting [' + id + ']: ' + url);
        let socket;
        try {
            socket = new WebSocket(url);
        } catch (e) {
            console.error('Failed to create WebSocket for ' + url + ': ' + e);
            setTimeout(function() { handleAttemptFailure(id); }, 0);
            return;
        }

        const timeout = setTimeout(function() {
            if (pendingAttempts.has(id)) {
                socket.onclose = null; socket.onerror = null; socket.onopen = null;
                try { socket.close(); } catch(e) {}
                handleAttemptFailure(id);
            }
        }, 5000);

        pendingAttempts.set(id, { url: url, proto: url.startsWith('wss') ? 'wss' : 'ws', isNative: false, socket: socket, timeout: timeout });

        socket.onopen = function() {
            if (!pendingAttempts.has(id)) return;
            const a = pendingAttempts.get(id);
            if (a && a.timeout) clearTimeout(a.timeout);
            handleAttemptWin(id);
        };

        socket.onclose = function(event) {
            if (id === winnerAttemptId) {
                handleSessionDisconnect(event.code, event.reason);
            } else if (pendingAttempts.has(id)) {
                handleAttemptFailure(id);
            }
        };

        socket.onerror = function() {
            // onclose will follow; let that drive the failure logic
        };

        socket.onmessage = function(event) {
            if (id !== winnerAttemptId) return;
            let msg;
            try {
                msg = JSON.parse(event.data);
            } catch (e) {
                console.error('Failed to parse message:', e);
                return;
            }
            // Deliberately separate from the parse failure above: a malformed frame and a bug
            // in a message handler need completely different fixes, and the handler case used
            // to be swallowed into console.error - invisible on Android, where there is no
            // console. Reported by type so the banner says which message broke.
            try {
                handleMessage(msg);
            } catch (e) {
                __clayShowError('handleMessage(' + (msg && msg.type ? msg.type : '?') +
                    ') threw: ' + __clayErrText(e));
            }
        };
    }

    // Cleanly close all pending attempts and the winner, then reconnect.
    // Every reconnect in the client funnels through here, so this is the one place worth
    // instrumenting. Instrumenting call sites individually is what left the real cause
    // invisible: `checkConnectionOnResume` was covered and reported a healthy socket every
    // time, while the reconnect actually came from one of the other eighteen callers - most
    // of which (the whole `visibilitychange` handler, including its own pong timeout) had no
    // reporting at all.
    //
    // The caller is derived from a stack trace rather than a passed-in reason so it cannot go
    // stale as callers are added, and so this needed no edit at nineteen sites.
    function reconnectCallerHint() {
        try {
            var lines = String(new Error().stack || '').split('\n');
            // 0 is this helper, 1 is forceReconnect itself; the first frame after those is the
            // caller. Firefox/JavaScriptCore omit the V8-style header line, so scan rather than
            // index blindly.
            for (var i = 0; i < lines.length; i++) {
                var f = lines[i].trim();
                if (!f || f.indexOf('reconnectCallerHint') !== -1 || f.indexOf('forceReconnect') !== -1) continue;
                if (f.indexOf('Error') === 0) continue;
                return f.replace(/^at\s+/, '').slice(0, 90);
            }
        } catch (e) { /* diagnostics must never break the path they report on */ }
        return '?';
    }

    function forceReconnect() {
        var now = Date.now();
        recordClientEvent('forceReconnect', 'ws=' + (ws ? ws.readyState : 'null')
            + ' auth=' + authenticated
            + ' sinceLastMs=' + (lastForceReconnectAt ? (now - lastForceReconnectAt) : -1)
            + ' from=' + reconnectCallerHint());
        if (now - lastForceReconnectAt < 1000) {
            var remaining = 1000 - (now - lastForceReconnectAt);
            debugLog('forceReconnect: debounced (' + (now - lastForceReconnectAt) + 'ms since last), retry in ' + remaining + 'ms');
            // A debounced call must not simply be dropped: if nothing else re-triggers a
            // reconnect (e.g. onResume() and a visibilitychange firing within the same
            // second — exactly this shape), the socket is left dead with no retry
            // scheduled at all. Coalesce instead of discarding: schedule one follow-up
            // call for when the debounce window clears, unless one's already pending.
            if (!forceReconnectRetryTimer) {
                forceReconnectRetryTimer = setTimeout(function() {
                    forceReconnectRetryTimer = null;
                    forceReconnect();
                }, remaining);
            }
            return;
        }
        if (forceReconnectRetryTimer) { clearTimeout(forceReconnectRetryTimer); forceReconnectRetryTimer = null; }
        lastForceReconnectAt = now;
        if (wakePongTimeout) { clearTimeout(wakePongTimeout); wakePongTimeout = null; }
        if (connectionTimeout) { clearTimeout(connectionTimeout); connectionTimeout = null; }

        pendingAttempts.forEach(function(attempt, id) {
            if (attempt.timeout) clearTimeout(attempt.timeout);
            if (attempt.isNative) {
                if (window.Android) try { window.Android.closeWebSocket(id); } catch(e) {}
            } else if (attempt.socket) {
                attempt.socket.onclose = null; attempt.socket.onerror = null;
                attempt.socket.onopen = null; attempt.socket.onmessage = null;
                try { attempt.socket.close(); } catch(e) {}
            }
        });
        pendingAttempts.clear();

        if (ws) {
            ws.onclose = null; ws.onerror = null;
            ws.onopen = null; ws.onmessage = null;
            if (winnerAttemptId !== null && window.Android) {
                try { window.Android.closeWebSocket(winnerAttemptId); } catch(e) {}
            }
            try { ws.close(); } catch(e) {}
            ws = null;
        }

        winnerAttemptId = null;
        connectionFailures = 0;
        keyAuthFailed = false;
        connectInProgress = false;
        authenticated = false;
        wakeStateCleared = false;

        Object.keys(worlds).forEach(function(k) {
            if (worlds[k]) worlds[k].connected = false;
        });
        updateStatusBar();

        var logList = document.getElementById('connection-log-list');
        if (logList) logList.innerHTML = '';
        var logRetryBtn = document.getElementById('connection-log-retry-btn');
        if (logRetryBtn) logRetryBtn.disabled = true;

        connect();
    }

    function connect() {
        if (connectInProgress) {
            debugLog('connect(): already in progress, skipping duplicate call');
            return;
        }

        // A stray connect() call (e.g. the resync fallback, app.js ~7945) must never orphan
        // a live, authenticated socket - only forceReconnect() is allowed to tear that down.
        if (authenticated && ws && ws.readyState === WebSocket.OPEN) {
            debugLog('connect(): already connected and authenticated, skipping');
            return;
        }

        if (window.Android && typeof window.Android.isSettingsConfigured === 'function') {
            if (!window.Android.isSettingsConfigured()) {
                openSettingsPopup('clay-server');
                return;
            }
        } else if (window.SKIP_CONNECT) {
            return;
        }

        // SSH-tunnel mode (Android only): verify the local tunnel process is actually up
        // before dialing it. If it's dead, Android kicks off a restart (fresh ephemeral
        // port) and pushes it to us via updateSshTunnelPort() -> forceReconnect(); defer
        // this cycle rather than hammering a dead port until the next watchdog event.
        if (window.SSH_MODE && window.Android &&
            typeof window.Android.ensureSshTunnelReady === 'function') {
            let tunnelReady = true;
            try { tunnelReady = window.Android.ensureSshTunnelReady(); } catch (e) {}
            if (!tunnelReady) {
                debugLog('connect(): SSH tunnel not ready, deferring (restart in progress)');
                // Match every other bail-out path in this function (see the setTimeout(connect, ...)
                // calls above) - without this, a restart that Android never gets around to kicking
                // off (or one whose updateSshTunnelPort() callback gets lost, e.g. to the
                // forceReconnect() debounce) leaves nothing to ever call connect() again.
                setTimeout(connect, 2000);
                return;
            }
        }

        // Starting a fresh cycle - a stale winner, leftover pending attempts, or leftover
        // connection-log rows from a prior cycle must not bleed into this one (avoids
        // spurious "(lost)" rows and the dialog re-appearing populated with stale entries
        // after a healthy reconnect). Normally pendingAttempts is already empty by now
        // (the winning/failing paths clear it), so this is just a defensive sweep.
        winnerAttemptId = null;
        if (pendingAttempts.size > 0) {
            pendingAttempts.forEach(function(a, aid) {
                if (a.timeout) clearTimeout(a.timeout);
                if (a.isNative) {
                    if (window.Android) try { window.Android.closeWebSocket(aid); } catch(e) {}
                } else if (a.socket) {
                    a.socket.onclose = null; a.socket.onerror = null;
                    a.socket.onopen = null; a.socket.onmessage = null;
                    try { a.socket.close(); } catch(e) {}
                }
            });
            pendingAttempts.clear();
        }
        var staleLogList = document.getElementById('connection-log-list');
        if (staleLogList) staleLogList.innerHTML = '';
        var staleLogRetryBtn = document.getElementById('connection-log-retry-btn');
        if (staleLogRetryBtn) staleLogRetryBtn.disabled = true;

        const candidates = buildCandidates();
        if (!candidates.length || !candidates[0].host) {
            openSettingsPopup('clay-server');
            return;
        }

        connectInProgress = true;
        if (shouldShowConnectionWindow()) showConnectionLog();

        candidates.forEach(function(candidate) {
            const id = nextAttemptId++;
            addConnectionAttempt(candidate.url, id);
            if (candidate.proto === 'wss' && hasNativeWebSocket()) {
                connectWithNativeWebSocket(id, candidate.url);
            } else {
                connectWithBrowserWebSocket(id, candidate.url);
            }
        });
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
                    hideConnectionLog();
                    hideReconnectModal();
                    elements.authError.textContent = '';
                    elements.input.focus();
                    // Update UI based on multiuser mode
                    updateMultiuserUI();
                    // Declare client type to server (Android when running inside the
                    // Android app's WebView bridge, Web for regular browser clients)
                    send({ type: 'ClientTypeDeclaration', client_type: window.Android ? 'Android' : 'Web' });
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
                    // Now that the socket is authenticated, report whatever lifecycle events
                    // Java buffered while there was nothing to report them on - including the
                    // onCreate that says whether this was a resume or a rebuild.
                    flushAndroidLifecycleEvents();
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
                    // Password changed or invalid — clear cached credential so we don't loop.
                    lastGoodPassword = null;
                    lastGoodUsername = null;
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
                    // Refresh the Modify Key dialog if it's the one that requested this
                    if (isModifyKeyDialogOpen()) {
                        renderModifyKeyDialog(document.getElementById('modify-key-dialog'));
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
                // Our ▶ ownership id for this session. Captured before the world hydration
                // below so lineIsNew() is already correct for the first render.
                myDisplayId = (typeof msg.your_display_id === 'number') ? msg.your_display_id : 0;
                // Preserve already-downloaded scrollback across a reconnect instead
                // of discarding it: the WebSocket may drop and reconnect (network
                // change, resume) while the JS heap survives (always true for
                // Android/webview backgrounding), so capture the in-memory buffers
                // from before this InitialState - by world name - while `worlds`
                // still refers to the old array. See requestGapFill()/
                // scheduleWorldCacheSave() above for the other half of this.
                //
                // `worlds.length > 0` (checked here, before reassignment) is the
                // signal for "do we have real prior session data worth preserving".
                // `worlds` itself is never cleared except by an explicit
                // user-initiated LoggedOut, never by a mere disconnect, so it
                // correctly distinguishes a live resync from a genuinely fresh
                // session - unlike a flag that gets reset on every disconnect
                // (which is what used to gate this, and silently broke both this
                // capture and world-focus preservation below on every reconnect).
                var priorWorldsByName = {};
                var priorCurrentWorldName = (worlds[currentWorldIndex] && worlds[currentWorldIndex].name) || null;
                var isResync = worlds.length > 0;
                if (isResync) {
                    worlds.forEach((w) => {
                        if (w && w.name && w.output_lines && w.output_lines.length > 0) {
                            priorWorldsByName[w.name] = w;
                        }
                    });
                }

                worlds = msg.worlds || [];

                // Which world should be focused after this InitialState? Resolved by
                // world name (the stable identity used elsewhere, e.g. the scrollback
                // cache) rather than index, since indices can shift if worlds were
                // added/removed on the server between sessions. Priority order:
                //  1. The world we were actually looking at a moment ago, in memory -
                //     covers any live reconnect where the JS heap survived (network
                //     blip, Android background/resume, SSH tunnel restart).
                //  2. The last world persisted to localStorage - covers a genuinely
                //     cold start (page reload, app/process restart) where nothing
                //     survived in memory. See persistLastActiveWorld() below.
                //  3. The server's current_world_index - the only sane default for a
                //     truly first-ever connection, where neither of the above exists.
                // The URL-lock mechanism (lockedWorldName, checked further down) still
                // force-switches after this and takes final precedence regardless.
                var resolvedWorldIndex = -1;
                if (isResync && priorCurrentWorldName) {
                    resolvedWorldIndex = worlds.findIndex((w) => w.name === priorCurrentWorldName);
                }
                if (resolvedWorldIndex < 0) {
                    var persistedWorldName = null;
                    try { persistedWorldName = localStorage.getItem(lastWorldStorageKey()); } catch (e) {}
                    if (persistedWorldName) {
                        resolvedWorldIndex = worlds.findIndex((w) => w.name === persistedWorldName);
                    }
                }
                if (resolvedWorldIndex < 0) {
                    resolvedWorldIndex = msg.current_world_index !== undefined ? msg.current_world_index : 0;
                }
                // Clamp defensively: an out-of-range index must never silently land on
                // an arbitrary REAL world via Math.min(x, worlds.length - 1) - that
                // previously mapped multiuser's current_world_index: 9999 sentinel
                // ("this user has no world connection yet", see
                // build_multiuser_initial_state in daemon.rs) straight onto whatever
                // happens to be the LAST world in the list, showing that world's
                // (unrelated) connection state instead of a well-defined starting
                // point. Fall back to the first world instead; worlds.length === 0 is
                // still handled by the `!world` guards in renderOutput()/
                // updateStatusBar() downstream.
                if (resolvedWorldIndex < 0 || resolvedWorldIndex >= worlds.length) {
                    resolvedWorldIndex = 0;
                }
                currentWorldIndex = resolvedWorldIndex;

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
                // Wrapped in try/catch: a single malformed/unexpected world entry (e.g. a
                // version mismatch between this client and an older/newer remote server)
                // must not throw out of the whole InitialState handler and skip the
                // renderOutput()/updateStatusBar() calls below - that would freeze the
                // page on its pre-connection state (blank world name, splash screen)
                // with no way to recover short of a manual reload.
                try {
                worlds.forEach((world, idx) => {
                    const rawPriorWorld = world.name ? priorWorldsByName[world.name] : null;
                    const rawCachedWorld = (!rawPriorWorld && world.name) ? worldCacheLoaded[world.name] : null;
                    // Server-restart detection: seq counters are only monotonic within a
                    // single server process (see WorldStateMsg.next_seq's doc comment in
                    // websocket.rs) - a real (>0) seq restart resets to 0. A cached/in-memory
                    // buffer that claims a real seq at or past what THIS fresh session has
                    // produced so far (world.next_seq) cannot belong to the current server
                    // run; it predates a restart. Trusting it anyway is exactly the bug this
                    // guards against: every subsequent live ServerData batch for this world
                    // would satisfy the "already seen" dedup check below (world._max_seq) and
                    // be silently dropped forever - output frozen at connect time, surviving
                    // even a manual resync, since RequestState re-runs this same hydration
                    // and lands on the same poisoned buffer again. `> 0` on the candidate's
                    // own max seq (not just `>=`) avoids a false positive when neither side
                    // has ever recorded a real seq yet (a brand-new world, or multiuser mode,
                    // where World.next_seq is always sent as a hardcoded 0 - see its doc
                    // comment in daemon.rs's InitialState builder).
                    const serverNextSeq = world.next_seq || 0;
                    const serverEpoch = world.seq_epoch || 0;

                    // Primary test: does this buffer's seq space still exist?
                    //
                    // `seq_epoch` is a random id the server mints when a world's sequence
                    // space starts and keeps (across hot reloads) for as long as it lasts. A
                    // buffer stamped with a different epoch holds seqs that refer to a space
                    // that is gone, so every dedup decision made from it is meaningless.
                    //
                    // This replaces the arithmetic test below as the real defence. That test
                    // compares a cached high-water mark against the server's current counter,
                    // and BOTH sides move: ordinary output - or an archive load fabricating
                    // seqs - can push `next_seq` back above a stale cached value, at which
                    // point the comparison falls silent while the cache is still poisoned.
                    // That is precisely how the v1.5.23-26 incident persisted: the server-side
                    // counter fix moved `next_seq` one past the cached mark and hid the
                    // detector from itself. Equality against a random id cannot do that.
                    //
                    // A cache with no epoch recorded predates this field. It is discarded
                    // rather than trusted - that is a single extra download per world on
                    // upgrade, and it is what finally clears a buffer poisoned by the older
                    // versions without anyone having to wipe app storage by hand.
                    const epochKnown = serverEpoch !== 0;
                    const priorWorldWrongEpoch = !!(epochKnown && rawPriorWorld &&
                        rawPriorWorld._seq_epoch !== serverEpoch);
                    const cachedWorldWrongEpoch = !!(epochKnown && rawCachedWorld &&
                        rawCachedWorld.seqEpoch !== serverEpoch);
                    if (priorWorldWrongEpoch || cachedWorldWrongEpoch) {
                        const had = priorWorldWrongEpoch ? rawPriorWorld._seq_epoch : rawCachedWorld.seqEpoch;
                        console.warn('Clay: seq epoch changed for world "' + (world.name || ('#' + idx)) +
                            '" - discarding ' + (priorWorldWrongEpoch ? 'in-memory' : 'cached') +
                            ' scrollback (buffer epoch ' + (had === undefined ? 'absent' : had) +
                            ', server epoch ' + serverEpoch + ')');
                        if (cachedWorldWrongEpoch && world.name) {
                            clearWorldCacheEntry(world.name);
                        }
                    }

                    // Retained fallback for a server that does not send an epoch (older
                    // build, or multiuser where every seq is a hardcoded 0). Redundant
                    // whenever the epoch is known, and deliberately kept: it costs one
                    // comparison and covers the peers the epoch cannot reach.
                    const priorWorldStale = priorWorldWrongEpoch || !!(!epochKnown && rawPriorWorld &&
                        rawPriorWorld._max_seq > 0 && rawPriorWorld._max_seq >= serverNextSeq);
                    const cachedWorldStale = cachedWorldWrongEpoch || !!(!epochKnown && rawCachedWorld &&
                        rawCachedWorld.maxSeq > 0 && rawCachedWorld.maxSeq >= serverNextSeq);
                    if ((priorWorldStale && !priorWorldWrongEpoch) || (cachedWorldStale && !cachedWorldWrongEpoch)) {
                        const staleSeq = priorWorldStale ? rawPriorWorld._max_seq : rawCachedWorld.maxSeq;
                        console.warn('Clay: server session reset detected for world "' + (world.name || ('#' + idx)) +
                            '" - discarding stale ' + (priorWorldStale ? 'in-memory' : 'cached') +
                            ' scrollback (had seq up to ' + staleSeq + ', server session is only at ' + serverNextSeq + ')');
                        if (cachedWorldStale && world.name) {
                            clearWorldCacheEntry(world.name);
                        }
                    }

                    // Second, independent reason to throw a buffer away: it was poisoned by
                    // the pre-1.5.23 archive bug (see bufferIsCorrupted). The restart test
                    // above cannot catch this - on an established world the fabricated
                    // archive seqs sit BELOW the live range, so maxSeq stays under next_seq
                    // and the entry reads as perfectly current.
                    //
                    // It has to be caught here or a damaged client stays damaged forever: a
                    // fixed server stops sending archive lines, but rebuildSeenRanges unions
                    // the carried ranges straight back in on every reconnect, so the poisoned
                    // frontier survives reconnects, resyncs and app updates alike. Wiping app
                    // storage by hand was the only cure, which is not something a user should
                    // have to know.
                    const priorWorldCorrupt = !priorWorldStale && !!rawPriorWorld &&
                        bufferIsCorrupted(rawPriorWorld.output_lines);
                    const cachedWorldCorrupt = !cachedWorldStale && !!rawCachedWorld &&
                        bufferIsCorrupted(rawCachedWorld.lines);
                    if (priorWorldCorrupt || cachedWorldCorrupt) {
                        console.warn('Clay: discarding corrupted scrollback for world "' +
                            (world.name || ('#' + idx)) + '" (' +
                            (priorWorldCorrupt ? 'in-memory' : 'cached') +
                            ') - archived lines or out-of-order seqs from the pre-1.5.23 ' +
                            'archive bug; re-downloading from the server');
                        if (cachedWorldCorrupt && world.name) {
                            clearWorldCacheEntry(world.name);
                        }
                    }

                    const priorWorld = (priorWorldStale || priorWorldCorrupt) ? null : rawPriorWorld;
                    const cachedWorld = (cachedWorldStale || cachedWorldCorrupt) ? null : rawCachedWorld;
                    // Whether this world was seeded from a local buffer (in-memory
                    // reconnect or persistent cache) rather than the server's
                    // freshly-sent slice - startBackfill() uses this to gap-fill
                    // instead of doing a full backfill for this world.
                    // Stamp the live world so an in-memory reconnect (priorWorld) can be
                    // epoch-checked the same way the persisted cache is.
                    world._seq_epoch = serverEpoch;
                    world._hydratedFromLocal = !!(priorWorld || (cachedWorld && cachedWorld.lines && cachedWorld.lines.length > 0));
                    // Was this world covered by the AuthRequest.resume we just sent THIS
                    // connection (resumeSentThisConnection, recorded by
                    // buildResumeAckListForAuthRequest at send time)? Deliberately NOT the
                    // old heuristic `priorWorld && contiguousFrontier(priorWorld) > 0` - that
                    // stayed true across a RequestState-driven resync (no resume list is
                    // ever sent for that path), permanently sticking _gapFillPending true
                    // with no server reply ever coming to clear it (the stuck-at-90%
                    // scrollback indicator bug). Matched by name AND index - a world whose
                    // server-assigned index shifted between the resume send and this
                    // InitialState (e.g. another world was added/removed concurrently) did
                    // not actually get replayed at ITS current index, so it must not be
                    // treated as resumed either. Only the in-memory reconnect case
                    // (priorWorld) qualifies - a cachedWorld hit (the cross-session
                    // IndexedDB cache) has no server-assigned index until this very
                    // InitialState arrives, so it structurally cannot have been part of
                    // resume and still needs the client-driven requestGapFill() fallback
                    // below. When true, the server is already about to push exactly the
                    // missing lines unprompted as ScrollbackLines (request_id: 0) -
                    // startBackfill() must not also request them itself (redundant round
                    // trip), but does need _gapFillPending set so that unprompted reply is
                    // handled as an append, not a prepend.
                    const resumeEntry = priorWorld && world.name ? resumeSentThisConnection.get(world.name) : undefined;
                    world._resumedFromServer = !!(priorWorld && resumeEntry && resumeEntry.index === idx);
                    // Delivered-seq ranges carried over from whichever source hydrates this
                    // world below (PROTOCOL-ROADMAP.md Phase C). Without this the record of
                    // what the server already delivered is silently dropped here (only
                    // output_lines/_max_seq/_oldest_seq used to be preserved), so a real
                    // unrecovered hole becomes invisible to contiguousFrontier() and
                    // therefore to the next AuthRequest.resume/PongCheck.acked, which would
                    // then wrongly tell the server "I have everything up to _max_seq".
                    // Unioned with the hydrated buffer's own seqs by rebuildSeenRanges()
                    // after this chain.
                    let carriedSeenRanges = null;
                    if (priorWorld) {
                        // Reconnect: keep what we already had in memory - it's at
                        // least as complete as the fresh InitialState's front-loaded
                        // slice, and losing it is exactly the bug this preserves.
                        // dedupBySeq (Step 10): a phantom-gap resume replay before this fix
                        // shipped could have appended duplicates into this exact buffer.
                        world.output_lines = dedupBySeq(priorWorld.output_lines);
                        carriedSeenRanges = priorWorld._seenRanges || null;
                    } else if (cachedWorld && cachedWorld.lines && cachedWorld.lines.length > 0) {
                        // Cold start / full reload with a persistent cache hit: seed
                        // from the cache, then gap-fill (see startBackfill()) to pick
                        // up whatever arrived on the server while we were gone.
                        //
                        // One-time migration: caches written before the display-time
                        // prefix change hold snapshot-baked "✨ " text (the server used
                        // to bake it into build_initial_state's output). Post-change
                        // snapshots never do, so this is a no-op from then on - strip
                        // it here rather than bumping the cache DB version, which
                        // would silently discard every user's cached scrollback.
                        // dedupBySeq (Step 10, same rationale as above): this persistent
                        // cache can already hold duplicates baked in before this fix shipped.
                        world.output_lines = dedupBySeq(cachedWorld.lines.map(l =>
                            (l && l.from_server === false && typeof l.text === 'string'
                                && l.text.startsWith(CLIENT_LINE_PREFIX))
                                ? Object.assign({}, l, { text: l.text.slice(CLIENT_LINE_PREFIX.length) })
                                : l));
                        // Restore the delivered-seq record persisted alongside this cache
                        // entry (scheduleWorldCacheSave) - same rationale as the priorWorld
                        // branch above. Entries written before Phase C hold the legacy
                        // {maxSeq, seqGaps} shape instead, which converts exactly; entries
                        // older still have neither, which means "no known holes".
                        carriedSeenRanges = cachedWorld.seenRanges
                            || seenRangesFromLegacyGaps(cachedWorld.maxSeq, cachedWorld.seqGaps);
                    } else if (world.output_lines_ts && world.output_lines_ts.length > 0) {
                        // Use output_lines_ts if available (has timestamps)
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
                    // Track whether this world's history is known exhausted
                    // (server returned fewer lines than requested) - stops phase 2
                    // from re-queuing a world that has nothing left to give.
                    world._backfill_exhausted = false;
                    world._gapFillPending = world._resumedFromServer;
                    // Arm/clear the watchdog for the unprompted replay this flag is waiting
                    // on - it carries request_id 0 and so has no registered timeout.
                    if (world._resumedFromServer) {
                        armUnpromptedReplayWatchdog(idx);
                    } else {
                        clearUnpromptedReplayWatchdog(idx);
                    }
                    // The recompute loop below and rebuildSeenRanges() both exclude lines
                    // explicitly marked
                    // _has_real_seq: false - an ephemeral ServerData message that never
                    // touched output_lines server-side (seq: 0, the "bypass dedup" sentinel
                    // used by ~50 system/command-reply broadcast sites) gets
                    // lineSeq = world.output_lines.length (an ARRAY INDEX, not a seq) when
                    // first appended by the live ServerData handler, which tags it
                    // _has_real_seq: false at that point. Without this guard, a single such
                    // line anywhere in the buffer poisons _oldest_seq to a tiny value, making
                    // the first before_seq backfill request return almost nothing and
                    // permanently marking the world's history "exhausted" - deep scrollback
                    // then silently stops working for that world (PROTOCOL-ROADMAP.md's
                    // seq-drift fix). Deliberately `!== false` rather than requiring an
                    // explicit `true`: lines from server-authoritative sources this handler
                    // populates directly (output_lines_ts, the legacy plain-string
                    // conversion, an older cache entry saved before this field existed) never
                    // set _has_real_seq at all, but their `seq` (when present) is always
                    // genuinely real - only the live handler's fake-index fallback ever
                    // explicitly sets `false`. The same guard inside rebuildSeenRanges is
                    // defensive: an index is always <= the real seq at that position, so it
                    // can't currently regress the high-water mark, but leaving it unguarded
                    // would be a landmine later.
                    if (world.output_lines.length > 0) {
                        let minSeq = Infinity;
                        for (const line of world.output_lines) {
                            if (line._has_real_seq !== false && line.seq !== undefined && line.seq < minSeq) minSeq = line.seq;
                        }
                        if (minSeq !== Infinity) world._oldest_seq = minSeq;
                    }
                    // Rebuild the delivered-seq record from the hydrated buffer, unioned
                    // with whatever the previous session knew (see carriedSeenRanges above).
                    // Also (re)sets world._max_seq.
                    rebuildSeenRanges(world, carriedSeenRanges);
                    // Initialize pending_count from server (for More indicator)
                    if (world.pending_count === undefined) world.pending_count = 0;
                    // Don't merge pending_lines - they stay on the server and are
                    // released via PgDn/Tab, then broadcast as ServerData.
                    // This avoids duplicate lines when pending is released.
                    // Use server's centralized unseen tracking - don't reset to 0
                    // world.unseen_lines comes from server, keep it as-is
                    // A world that has genuinely connected before (was_connected) can end
                    // up with a stale showing_splash: true alongside its real accumulated
                    // output_lines (e.g. the server flag never got cleared before this
                    // client's first connection) - that must not hide already-existing
                    // output behind the splash screen (WebView/Android only render the
                    // splash image, see renderOutput()) until the user happens to switch
                    // worlds and back. The was_connected check is required: a fresh,
                    // never-connected world ALSO has showing_splash: true with non-empty
                    // output_lines (the server puts the 12-line ASCII splash art directly
                    // into output_lines, see World::new_with_splash in main.rs) - without
                    // this check, every startup splash gets wrongly cleared and the
                    // WebView's centered PNG logo falls through to plain left-aligned text.
                    if (world.showing_splash && world.was_connected && world.output_lines && world.output_lines.length > 0) {
                        world.showing_splash = false;
                    }
                });
                } catch (e) {
                    console.error('Clay: error normalizing InitialState worlds - continuing with partial state', e);
                }
                // Wrapped in try/catch for the same reason as the worlds.forEach above -
                // one unexpected/missing settings field must not prevent the world name
                // and output from ever rendering.
                try {
                if (msg.settings) {
                    if (msg.settings.input_height) {
                        setInputHeight(msg.settings.input_height);
                    }
                    if (msg.settings.more_mode_enabled !== undefined) {
                        moreModeEnabled = msg.settings.more_mode_enabled;
                    }
                    if (msg.settings.show_tags !== undefined) {
                        showTags = msg.settings.show_tags;
                        updateTagsTileState();
                    }
                    if (msg.settings.ansi_music_enabled !== undefined) {
                        ansiMusicEnabled = msg.settings.ansi_music_enabled;
                    }
                    if (msg.settings.zwj_enabled !== undefined) {
                        zwjEnabled = msg.settings.zwj_enabled;
                    }
                    if (msg.settings.tts_mode !== undefined) ttsMode = msg.settings.tts_mode;
                    if (msg.settings.tts_speak_mode !== undefined) ttsSpeakMode = msg.settings.tts_speak_mode;
                    if (msg.settings.tabs !== undefined) applyTabsMode(msg.settings.tabs);
                    if (msg.settings.icon_bar !== undefined) applyIconBarMode(msg.settings.icon_bar);
                    if (msg.settings.new_line_indicator !== undefined) {
                        newLineIndicator = msg.settings.new_line_indicator;
                    }
                    if (msg.settings.keyboard_always_visible !== undefined) {
                        keyboardAlwaysVisible = msg.settings.keyboard_always_visible;
                    }
                    applyKeyboardForceState();
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
                    if (msg.settings.scrollback_enabled !== undefined) {
                        scrollbackEnabled = msg.settings.scrollback_enabled;
                    }
                    if (msg.settings.log_input_enabled !== undefined) {
                        logInputEnabled = msg.settings.log_input_enabled;
                    }
                    if (msg.settings.dictionary_path !== undefined) {
                        dictionaryPath = msg.settings.dictionary_path;
                    }
                    if (msg.settings.spell_check_enabled !== undefined) {
                        spellCheckEnabled = msg.settings.spell_check_enabled;
                    }
                    // Web settings (web_secure no longer read — see declaration above)
                    if (msg.settings.http_enabled !== undefined) {
                        httpEnabled = msg.settings.http_enabled;
                    }
                    if (msg.settings.http_port !== undefined) {
                        httpPort = msg.settings.http_port;
                    }
                    if (msg.settings.web_path !== undefined) {
                        webPath = msg.settings.web_path;
                        // Auto-learn: a bundled-asset page (e.g. Android WebView) never
                        // gets {{WEB_PATH}} substituted by the server. Adopt the real
                        // value from the settings payload so basePath()/wsPathCandidates()
                        // use it immediately, and persist it via the Android bridge (if
                        // present) so future launches inject it up front.
                        window.WEB_PATH = msg.settings.web_path;
                        try {
                            if (window.Android && typeof window.Android.saveWebPath === 'function') {
                                window.Android.saveWebPath(msg.settings.web_path);
                            }
                        } catch (e) {
                            console.error('Error saving web path:', e);
                        }
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
                    if (msg.settings.wrapspace !== undefined) {
                        wrapspace = msg.settings.wrapspace;
                        applyWrapspace(wrapspace);
                    }
                    if (msg.settings.remote_initial_lines !== undefined) {
                        remoteInitialLines = msg.settings.remote_initial_lines;
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
                    settingsSynced = true;
                }
                } catch (e) {
                    console.error('Clay: error applying InitialState settings - continuing with partial state', e);
                }
                // Calculate activity count from world data (don't wait for ActivityUpdate message) -
                // needed immediately here since some InitialState-sending paths (ImportSettings,
                // hot-reload) never follow up with a broadcast_activity() call.
                serverActivityCount = worlds.filter((w, i) => i !== currentWorldIndex && worldHasActivity(w)).length;
                renderOutput();
                updateStatusBar();
                // Send initial view state for synchronized more-mode
                sendViewStateIfChanged();
                // Warn once per session if this client's bundled version differs from the
                // server's (only GUI-remote/Android can genuinely drift - see plan doc).
                // The local side goes through clientVersion(), which already treats an
                // unreplaced "{{...}}" template placeholder as absent rather than as a real
                // version (that has escaped once before); the same guard is applied inline to
                // the server-supplied value. Wrapped in its own try/catch so a
                // malformed/missing field can never break InitialState processing.
                try {
                    var localVersion = clientVersion();
                    var remoteVersion = msg.server_version;
                    if (!versionMismatchShown && localVersion && remoteVersion &&
                        remoteVersion.indexOf('{{') === -1 &&
                        localVersion !== remoteVersion) {
                        appendClientLine(
                            `Version mismatch: ${localVersion} (local) ≠ ${remoteVersion} (remote).`,
                            currentWorldIndex, 'system'
                        );
                        versionMismatchShown = true;
                    }
                } catch (e) {
                    console.error('Clay: error checking client/server version mismatch', e);
                }
                // Schedule lazy backfill of remaining scrollback history
                startBackfill();
                // The resume list is consumed by EXACTLY ONE InitialState - the one the
                // server sends in reply to the AuthRequest that carried it. Clear it here,
                // now that startBackfill() has read it, rather than waiting for the socket
                // to close.
                //
                // This is the fix for "Android is missing output at the bottom after
                // waking". A RequestState resync arrives on a still-open, already-
                // authenticated socket: no close, no AuthRequest, no resume list, and the
                // server sends NO unprompted replay for it (only AuthRequest.resume
                // triggers one). But this map used to survive until close, so that resync's
                // InitialState still found entries in it and set _resumedFromServer = true
                // for every world - which skips requestGapFill() AND sets _gapFillPending,
                // excluding the world from both backfill queues. The client then fetched
                // nothing newer and sat at whatever it had before the wake, with
                // _gapFillPending stuck true forever. RequestState is the dominant Android
                // post-wake path, so it recurred on every wake until the socket dropped.
                //
                // Keying on this map (rather than the older `priorWorld &&
                // contiguousFrontier(priorWorld) > 0` heuristic) was already meant to fix
                // exactly this - but a map that outlives its one use behaves identically to
                // that heuristic. The lifetime is the fix, not the key.
                resumeSentThisConnection = new Map();
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

                // Note editor mode: this is a dedicated note window/tab (desktop
                // GUI's own OS window, or the ?note= browser tab) - replace the
                // whole page view with the note editor. See enterNoteMode() for
                // the Android in-place equivalent (entered at runtime, not here).
                if (noteMode) {
                    enterNoteMode(noteMode.world_index);
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
                        world._seenRanges = [];
                        if (msg.world_index === currentWorldIndex) {
                            elements.output.innerHTML = '';
                            linesSincePause = 0;
                            paused = false;
                            pendingLines = [];
                        }
                    }
                    if (msg.data) {
                        // Dedup by EXACT delivered-seq membership (PROTOCOL-ROADMAP.md
                        // Phase C). A batch is a genuine duplicate only when every seq it
                        // spans has already been delivered to us; anything else carries at
                        // least one line we've never had and must be accepted.
                        //
                        // This is the fix for the one-way ratchet described at the top of
                        // this file: the old test dropped the WHOLE batch whenever its seq
                        // was <= world._max_seq and it didn't overlap a *recorded* gap. A
                        // high-water mark that had run ahead of the buffer - by any of the
                        // server-side ordering bugs found so far, or one not yet found -
                        // therefore ate that world's live output permanently, and no resync
                        // could recover it because the replay tripped the same test.
                        //
                        // Prefer the server-authoritative end_seq over a locally-approximated
                        // line count when the sender provided one - the approximation can
                        // undercount relative to the server's real batch span (e.g. a trailing
                        // partial line folded into the next batch), which would understate
                        // which seqs this batch actually covers.
                        const hasBatchSeq = msg.seq !== undefined && (msg.seq > 0 || msg.end_seq !== undefined);
                        let batchEndApprox = msg.seq;
                        let isGapFill = false;
                        if (hasBatchSeq) {
                            const approxLineCount = msg.data.split(/\r\n|\n|\r/).length;
                            batchEndApprox = msg.end_seq !== undefined ? msg.end_seq : msg.seq + Math.max(approxLineCount - 1, 0);
                            if (hasSeenRange(world, msg.seq, batchEndApprox)) {
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
                            // Belongs mid-buffer rather than at the tail: splice it into seq
                            // order below instead of rendering it as fresh tail output.
                            isGapFill = batchEndApprox < maxSeenSeq(world);
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
                        const gapFillLineObjs = []; // only populated when isGapFill
                        // rawIdx (this line's position in the FULL split batch, before any of
                        // the filters below) is what lineSeq must be derived from, not a
                        // running count of lines actually kept - the server assigned one real
                        // seq per line in the original batch regardless of what this client
                        // later decides to filter for display (ANSI-only lines, idler markers,
                        // grep mode). Deriving seq from a post-filter count let every filtered
                        // line permanently drift _max_seq below the server's true high-water
                        // seq, which then produced phantom gaps and duplicate lines on the next
                        // reconnect (see PROTOCOL-ROADMAP.md's seq-drift fix).
                        rawLines.forEach((line, rawIdx) => {
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
                            // msg.seq === 0 is a legitimate real seq (a world's very first
                            // line, see next_seq's initial value server-side) - end_seq being
                            // present is what distinguishes "seq: 0, a real value" from
                            // "seq field absent/defaulted", so a real end_seq widens this too.
                            const hasRealSeq = msg.seq !== undefined && (msg.seq > 0 || msg.end_seq !== undefined);
                            const lineSeq = hasRealSeq ? msg.seq + rawIdx : (isGapFill ? -1 : world.output_lines.length);
                            // Per-line duplicate skip (Phase C). The batch as a whole passed
                            // the all-seen check above, but it can still partially overlap
                            // what we already hold - a resync replay that straddles the edge
                            // of our buffer, or a retransmit that extends one. Skipping only
                            // the seqs we've genuinely already got is what lets the rest
                            // through, instead of the old all-or-nothing batch drop.
                            if (hasRealSeq && hasSeenSeq(world, lineSeq)) {
                                return;
                            }
                            // highlight_colors is parallel to the PRE-filter split, so index
                            // by rawIdx (same reasoning as lineSeq above). Absent/empty means
                            // nothing in this batch was highlighted.
                            const lineHighlight = (Array.isArray(msg.highlight_colors) && msg.highlight_colors.length > rawIdx)
                                ? msg.highlight_colors[rawIdx] : null;
                            // `viewed` mirrors OutputLine::viewed, which add_output sets to
                            // "was anybody watching this world when this arrived". Carrying it
                            // is what lets claimUnviewedLocally() predict the server's claim
                            // instead of guessing every line is unviewed - without it, lines
                            // that arrived while the console (or another client) was watching
                            // would be optimistically marked ▶ and then taken back.
                            const lineObj = { text: truncateIfNeeded(line), ts: lineTs, seq: lineSeq, from_server: isFromServer, _has_real_seq: hasRealSeq, gagged: msg.gagged || false, highlight_color: lineHighlight, viewed: !!msg.is_viewed };

                            if (isGapFill) {
                                // This batch fills a historical hole, not the tail — collect it
                                // and splice it into output_lines in seq order below instead of
                                // rendering it incrementally as if it were new tail output.
                                gapFillLineObjs.push(lineObj);
                                appendedLineCount++;
                                return;
                            }

                            const lineIndex = world.output_lines.length;
                            world.output_lines.push(lineObj);
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
                                // A live arrival is never owned by anyone (ownership is only
                                // assigned when a client displays a world), so this is false
                                // here today - evaluated anyway so the incremental-append path
                                // stays consistent with a full renderOutput().
                                const lineMarkedNew = lineIsNew(lineObj, world);
                                // Gagged lines are stored but not rendered (only visible with F2)
                                // They bypass more-mode entirely
                                if (msg.gagged) {
                                    // Don't render or count for more-mode
                                } else if (!hasRealSeq && isFromServer) {
                                    // Released pending lines (seq=0, from_server=true) bypass local
                                    // more-mode to avoid flickering the More indicator
                                    appendNewLine(line, lineTs, msg.world_index, lineIndex, lineMarkedNew, isFromServer, lineHighlight);
                                } else {
                                    handleIncomingLine(line, lineTs, msg.world_index, lineIndex, lineMarkedNew, isFromServer, lineHighlight);
                                }
                            }
                            // Note: Don't track unseen_lines locally - server handles centralized tracking
                            // and sends UnseenUpdate messages to keep all clients in sync
                        });
                        // Record the batch's TRUE delivered span - the server-authoritative
                        // end_seq when present, else rawLines.length (the full PRE-FILTER
                        // batch size, matching the rawIdx-based lineSeq above) rather than
                        // appendedLineCount. This must run whenever the batch has a real seq,
                        // NOT only when appendedLineCount > 0: a batch that's entirely
                        // filtered out client-side (a lone idler/gagged line, or every line
                        // stripped by grep mode) still consumed real seqs server-side. Marking
                        // only the lines we kept would punch phantom holes and make us
                        // re-request data we were already sent (PROTOCOL-ROADMAP.md's
                        // seq-drift fix, and see _seenRanges' doc comment).
                        if (hasBatchSeq) {
                            const batchEndSeq = msg.end_seq !== undefined ? msg.end_seq : msg.seq + rawLines.length - 1;
                            markSeqRangeSeen(world, msg.seq, batchEndSeq);
                            world._max_seq = maxSeenSeq(world);
                        }
                        if (isGapFill) {
                            if (gapFillLineObjs.length > 0) {
                                insertLinesBySeq(world, gapFillLineObjs);
                                console.warn('RECOVERED out-of-order ServerData (filled a gap):', { world_index: msg.world_index, seq: msg.seq, count: appendedLineCount });
                                send({
                                    type: 'ReportOutOfOrder',
                                    world_index: msg.world_index,
                                    line_seq: msg.seq,
                                    recovered_count: appendedLineCount,
                                    source: window.Android ? 'android' : 'web'
                                });
                                // These lines were inserted earlier than the tail — an incremental
                                // append can't place them correctly, so re-render fully.
                                if (msg.world_index === currentWorldIndex) {
                                    renderOutput();
                                }
                            }
                        }
                        if (msg.world_index !== currentWorldIndex) {
                            updateStatusBar();
                        }
                        // After flush, force full re-render to ensure output is visible
                        // (handles case where splash image was re-rendered by WorldConnected)
                        if (msg.flush && msg.world_index === currentWorldIndex) {
                            renderOutput();
                        }
                        if (appendedLineCount > 0) scheduleWorldCacheSave(msg.world_index);
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

            case 'CertMismatch':
                // The MUD server's TLS certificate no longer matches the
                // trust-on-first-use pin in ~/.clay/known_hosts.dat. The
                // connection was blocked server-side; show old vs new
                // fingerprints and offer to trust the new certificate.
                showCertMismatchDialog(msg.world_index, msg.host, msg.old_fingerprint, msg.new_fingerprint);
                break;

            case 'ImportNeedsInsecureConfirm':
                // /import's target didn't accept TLS; ask before resending the
                // password/auth-key over a plaintext ws:// connection.
                showImportInsecureConfirmDialog(msg.addr);
                break;

            case 'ImportResult':
                appendClientLine(msg.summary, currentWorldIndex, 'system');
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
                    // Seed the delivered-seq record from the lines this message carried
                    // (PROTOCOL-ROADMAP.md Phase C). Without this the world starts claiming
                    // nothing delivered, so the first switch-time/audit check would ask the
                    // server to re-send history we were just handed. Also sets _max_seq.
                    rebuildSeenRanges(world, null);
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
                        updateTagsTileState();
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
                    if (msg.settings.tabs !== undefined) applyTabsMode(msg.settings.tabs);
                    if (msg.settings.icon_bar !== undefined) applyIconBarMode(msg.settings.icon_bar);
                    if (msg.settings.new_line_indicator !== undefined) {
                        const oldNli = newLineIndicator;
                        newLineIndicator = msg.settings.new_line_indicator;
                        if (oldNli !== newLineIndicator) {
                            renderOutput();
                        }
                    }
                    if (msg.settings.keyboard_always_visible !== undefined) {
                        keyboardAlwaysVisible = msg.settings.keyboard_always_visible;
                    }
                    applyKeyboardForceState();
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
                    if (msg.settings.scrollback_enabled !== undefined) {
                        scrollbackEnabled = msg.settings.scrollback_enabled;
                    }
                    if (msg.settings.log_input_enabled !== undefined) {
                        logInputEnabled = msg.settings.log_input_enabled;
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
                    // Web settings (web_secure no longer read — see declaration above)
                    if (msg.settings.http_enabled !== undefined) {
                        httpEnabled = msg.settings.http_enabled;
                    }
                    if (msg.settings.http_port !== undefined) {
                        httpPort = msg.settings.http_port;
                    }
                    if (msg.settings.web_path !== undefined) {
                        webPath = msg.settings.web_path;
                        // Auto-learn (see InitialState handler above for rationale).
                        window.WEB_PATH = msg.settings.web_path;
                        try {
                            if (window.Android && typeof window.Android.saveWebPath === 'function') {
                                window.Android.saveWebPath(msg.settings.web_path);
                            }
                        } catch (e) {
                            console.error('Error saving web path:', e);
                        }
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
                    if (msg.settings.wrapspace !== undefined) {
                        wrapspace = msg.settings.wrapspace;
                        applyWrapspace(wrapspace); // pure CSS reflow, no re-render needed
                    }
                    if (msg.settings.remote_initial_lines !== undefined) {
                        const remoteLinesChanged = remoteInitialLines !== msg.settings.remote_initial_lines;
                        remoteInitialLines = msg.settings.remote_initial_lines;
                        // Re-trim the persistent scrollback cache to the new cap right
                        // away (scheduleWorldCacheSave reads remoteInitialLines fresh
                        // when it actually writes) rather than waiting for the next
                        // line of output to happen to trigger a save - otherwise a
                        // lowered setting wouldn't shrink an existing cache until a
                        // world got new traffic.
                        if (remoteLinesChanged) {
                            worlds.forEach((w, idx) => { if (w && w.name) scheduleWorldCacheSave(idx); });
                        }
                        // Recompute backfillTotalTarget live (PROTOCOL-ROADMAP.md's
                        // scrollback-reachability fix) - previously this was only ever
                        // computed once, inside startBackfill() at connect time, so raising
                        // Remote Lines without reconnecting had no visible effect: the
                        // extra history was never fetched (backfillInProgress had already
                        // finished and nothing restarted the pump against the new target).
                        // A backfill genuinely still IN PROGRESS doesn't need special
                        // handling here - it already reads backfillTotalTarget fresh on
                        // every check (the ScrollbackLines handler's phase-2 requeue test),
                        // so it naturally picks up the new value on its own.
                        if (remoteLinesChanged) {
                            backfillTotalTarget = Math.max(remoteInitialLines || 100, backfillPhase1Target);
                            // _backfill_exhausted reflects the server's own "no more history"
                            // signal (backfill_complete), which is usually a genuine
                            // exhaustion - but it can also be an artifact of a since-fixed
                            // poisoned _oldest_seq (a tiny before_seq request that returned
                            // almost nothing and got misread as exhaustion, see the
                            // _has_real_seq guards in the InitialState handler). Clearing it
                            // here means worst case one extra wasted round-trip per
                            // genuinely-exhausted world, which just re-sets the flag.
                            worlds.forEach((w) => { if (w) w._backfill_exhausted = false; });
                            if (!backfillInProgress) {
                                const anyWorldNeedsMore = worlds.some((w) => {
                                    const received = w && w.output_lines ? w.output_lines.length : 0;
                                    return received < backfillTotalTarget;
                                });
                                if (anyWorldNeedsMore) {
                                    backfillInProgress = true;
                                    startBackfillPhase2();
                                }
                            }
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
                    settingsSynced = true;
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
                    if (wakeStateCleared) {
                        // We cleared world states before the ping — connection is alive,
                        // restore auth and request fresh world state from the server
                        wakeStateCleared = false;
                        authenticated = true;
                        ws.send(JSON.stringify({ type: 'RequestState' }));
                    } else {
                        sendViewStateIfChanged();
                    }
                }
                break;

            case 'PingCheck':
                // Server liveness check for /remote command - respond immediately, and
                // piggyback our current per-world ack (PROTOCOL-ROADMAP.md Step 5) so this
                // reply also counts as a fresh ack even if the periodic keepalive ack
                // (see the setInterval below) hasn't fired recently.
                send({ type: 'PongCheck', nonce: msg.nonce || 0, acked: buildResumeAckList() });
                break;

            case 'ActionsUpdated':
                actions = msg.actions || [];
                if (actionsListPopupOpen) {
                    renderActionsList();
                }
                renderIconBar();
                break;

            case 'CalculatedWorld':
                // Server calculated next/prev world - switch to it
                if (msg.index !== null && msg.index !== undefined && msg.index !== currentWorldIndex) {
                    switchWorldLocal(msg.index);
                }
                break;

            case 'NoteEditorState':
                // Reply to RequestNoteEditorState (see noteMode handling above) —
                // populate the note-editor window/tab with this world's current notes.
                if (elements.noteEditorTextarea) {
                    elements.noteEditorTextarea.value = msg.notes || '';
                }
                if (elements.noteEditorTitle) {
                    elements.noteEditorTitle.textContent = 'Notes: ' + msg.world_name;
                }
                document.title = 'Clay - Notes: ' + msg.world_name;
                break;

            case 'UnseenCleared':
                // Another client (console, web, or GUI) has viewed this world
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    worlds[msg.world_index].unseen_lines = 0;
                    updateStatusBar();
                }
                break;

            case 'ClaimedNew':
                // The server handed US ownership of exactly these lines' ▶ markers (we just
                // started displaying that world). Sent to this client only - a claim never
                // changes any other client's rendering. An explicit seq list, not a range:
                // unviewed lines are not a contiguous tail (see WsMessage::ClaimedNew).
                //
                // Also the reconciliation point for claimUnviewedLocally()'s optimistic
                // claim: the server's list is authoritative, so a seq we guessed at but that
                // is missing from it gets its marker taken back. Sent even when the list is
                // empty (see App::claim_world_for) precisely so a wrong guess is always
                // corrected. The re-render is skipped when nothing actually changed, which is
                // the common case now - that skipped repaint is the ▶ "flash" going away.
                if (msg.world_index !== undefined && worlds[msg.world_index] && Array.isArray(msg.seqs)) {
                    const cWorld = worlds[msg.world_index];
                    const granted = new Set(msg.seqs);
                    const guess = cWorld._optimisticClaim;
                    cWorld._optimisticClaim = null;
                    // Only a *fresh* guess may be revoked. Every path that claims optimistically
                    // is answered promptly (MarkWorldSeen and ClientVisibility both always reply,
                    // the latter even when the server already considered us visible), so a guess
                    // older than one round trip means nothing answered it and this ClaimedNew is
                    // about something else entirely.
                    const guessed = (guess && (Date.now() - guess.at) < OPTIMISTIC_CLAIM_TTL_MS)
                        ? guess.seqs : null;
                    let changed = false;
                    for (const line of (cWorld.output_lines || [])) {
                        if (granted.has(line.seq)) {
                            if (line.display_id !== myDisplayId) changed = true;
                            line.display_id = myDisplayId;
                            line.viewed = true;
                        } else if (guessed && guessed.has(line.seq) && line.display_id === myDisplayId) {
                            line.display_id = null;
                            changed = true;
                        }
                    }
                    if (changed && msg.world_index === currentWorldIndex) {
                        renderOutput();
                    }
                }
                break;

            case 'ReleasedNew':
                // Our own markers on that world are cleared (we switched away, hit Ctrl+L, or
                // backgrounded). Other clients' markers live on their own lines' display_id
                // and are unaffected - that is the whole point of per-line ownership.
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    // Drops any outstanding optimistic guess too: the server has just told us
                    // we own nothing here, which supersedes whatever we predicted.
                    worlds[msg.world_index]._optimisticClaim = null;
                    for (const line of (worlds[msg.world_index].output_lines || [])) {
                        if (line.display_id === myDisplayId) line.display_id = null;
                    }
                    if (msg.world_index === currentWorldIndex) {
                        renderOutput();
                    }
                }
                break;

            case 'NotesChanged':
                // Notes for a world were saved (from this client, another
                // client, or the console) - update the note icon's visibility.
                if (msg.world_index !== undefined && worlds[msg.world_index] && worlds[msg.world_index].settings) {
                    worlds[msg.world_index].settings.has_notes = !!msg.has_notes;
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

            case 'PausedState': {
                const el = document.getElementById('session-paused-indicator');
                if (el) el.style.display = msg.paused ? 'flex' : 'none';
                break;
            }

            case 'ShowTagsChanged':
                // Server toggled show_tags (F2 or /tag command)
                showTags = msg.show_tags;
                updateTagsTileState();
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
                // A gap-fill can come back short because the server clamped it against this
                // world's pending backlog - more was owed, just not deliverable yet. The
                // server reports that as backfill_complete: false so _gapFillPending stays
                // armed; now that the backlog has released, re-drive it.
                if (msg.world_index !== undefined && worlds[msg.world_index]
                    && worlds[msg.world_index]._gapFillPending) {
                    worlds[msg.world_index]._gapFillPending = false;
                    requestGapFill(msg.world_index);
                }
                break;

            case 'ExecuteLocalCommand':
                // Server wants us to execute a command locally (from action)
                if (msg.command) {
                    executeLocalCommand(msg.command);
                }
                break;

            case 'OpenWindow':
                // In WebView mode, use IPC to spawn a new native WebView window
                // (window.open doesn't produce a usable new window there).
                if (window.WEBVIEW_MODE) {
                    sendIpc('new-window:' + (msg.world || ''));
                } else {
                    var worldParam = msg.world ? '?world=' + encodeURIComponent(msg.world) : '';
                    // Use WS_HOST/WS_PORT if available (WebView uses custom protocol, not real host)
                    var wsProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                    var wsHost = window.WS_HOST || window.location.hostname;
                    var wsPort = (window.WS_PORT && window.WS_PORT !== 0)
                        ? window.WS_PORT : window.location.port;
                    var openUrl = wsProto + '://' + wsHost + ':' + wsPort + basePath() + '/' + worldParam;
                    window.open(openUrl, '_blank');
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

            case 'ConnectionsListResponse':
                // addRawOutputLines stores these as from_server: false, so
                // appendNewLine/renderOutput add the client-generated marker
                // at display time - same as the console does.
                addRawOutputLines(msg.lines || [], currentWorldIndex);
                redisplayCurrentPrompt();
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
                // Switch-time delivery check (PROTOCOL-ROADMAP.md Phase C). switchWorldLocal()
                // sends RequestWorldState on every world switch, and the server now reports
                // that world's authoritative deliverable_high_seq on the reply - so the exact
                // moment the user looks at a world is also the moment we verify we actually
                // have all of it. Previously nothing checked here at all: SwitchWorld sends no
                // content and the client renders straight from its local buffer, so a world
                // that quietly lost lines while unviewed just looked empty. Deliberately
                // outside the currentWorldIndex guard below - correctness of the buffer
                // doesn't depend on which world happens to be focused when the reply lands.
                if (worlds[msg.world_index] && msg.deliverable_high_seq !== undefined) {
                    const w = worlds[msg.world_index];
                    const have = contiguousFrontier(w);
                    // have > 0: a world we hold nothing for is startBackfill()'s job, not a
                    // gap-fill's (asking after_seq: 0 would pull the whole server-side ring).
                    if (have > 0 && have < msg.deliverable_high_seq && !w._gapFillPending) {
                        console.warn('World switch: behind server, requesting gap-fill', {
                            world_index: msg.world_index, have: have, server: msg.deliverable_high_seq
                        });
                        requestGapFill(msg.world_index, have);
                    }
                }
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
                    const previousWorldIndex = currentWorldIndex;
                    // Clear new line indicators on the world we're LEAVING - previously only
                    // switchWorldLocal did this, so cycling worlds (Escape+w/Alt+w, which
                    // reply with WorldSwitchResult rather than going through
                    // switchWorldLocal) left stale ▶ markers behind on the world being left.
                    // No-op when the result is for the world we're already on (e.g. only one
                    // cycleable world), same as clearLeavingWorldState always was.
                    if (msg.world_index !== previousWorldIndex) {
                        clearLeavingWorldState(previousWorldIndex);
                    }
                    currentWorldIndex = msg.world_index;
                    if (worlds[msg.world_index]) {
                        worlds[msg.world_index].pending_count = msg.pending_count || 0;
                        worlds[msg.world_index].paused = msg.paused || false;
                    }
                    updateStatusBar();
                    // Same reason as in switchWorldLocal: claim before the paint so ▶ is
                    // there on the first frame rather than after MarkWorldSeen round-trips.
                    claimUnviewedLocally(msg.world_index);
                    renderOutput();
                    // Send MarkWorldSeen since we're now viewing this world; tell the server
                    // which world we left so it can clear that world's indicators even across
                    // a reconnect (see MarkWorldSeen's doc comment in websocket.rs).
                    send({
                        type: 'MarkWorldSeen',
                        world_index: currentWorldIndex,
                        previous_world_index: previousWorldIndex
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
                            from_archive: line.from_archive || false
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

            case 'ResyncRequired':
                // Live-connection resync (PROTOCOL-ROADMAP.md Step 3/5): our outbound
                // channel on the server overflowed and a ServerData batch for this world
                // may have been dropped before ever reaching us - distinct from the
                // reconnect path (AuthRequest.resume) above, this fires on an otherwise
                // healthy, still-open connection. Pull exactly what was missed via the
                // same gap-fill machinery reconnect uses (RequestScrollback/
                // ScrollbackLines, see requestGapFill()/ScrollbackLines handler below)
                // rather than duplicating that logic here.
                if (msg.world_index !== undefined && worlds[msg.world_index]) {
                    requestGapFill(msg.world_index, msg.from_seq);
                }
                break;

            case 'ScrollbackLines': {
                if (msg.world_index === undefined || !worlds[msg.world_index]) break;
                const world = worlds[msg.world_index];

                // Resolve which outstanding request this reply answers, and route on THAT
                // instead of the ambiguous world._gapFillPending flag alone (the stuck-at-90%
                // scrollback indicator bug, PROTOCOL-ROADMAP.md's seq-drift fix Bug 2): a
                // before_seq backfill reply landing while _gapFillPending happened to be true
                // (e.g. stuck true after a RequestState resync that never sent a resume list)
                // used to be misrouted into the gap-fill branch, which silently dropped its
                // lines (they're older, so isNew is false and no gap overlaps) and `break`d
                // before ever advancing the pump - leaving backfillInProgress stuck forever.
                // request_id 0 is reserved for a server-initiated unprompted resume replay
                // (never registered - the client didn't ask for it); a registered id names
                // its own recorded kind; anything else (undefined, or unrecognized - an old
                // server, or a request this client didn't itself track) falls back to the
                // legacy heuristic so behavior is unchanged against a server that predates
                // this field.
                let kind;
                if (msg.request_id === 0) {
                    kind = 'gapfill';
                    // The unprompted replay we were waiting on has arrived - disarm its
                    // watchdog so it can't fire later and issue a redundant gap-fill.
                    clearUnpromptedReplayWatchdog(msg.world_index);
                } else if (msg.request_id !== undefined && pendingScrollbackRequests.has(msg.request_id)) {
                    const entry = resolveScrollbackRequest(msg.request_id);
                    if (entry.worldIndex !== msg.world_index) {
                        console.warn('ScrollbackLines request_id/world_index mismatch', { request_id: msg.request_id, expected: entry.worldIndex, got: msg.world_index });
                    }
                    kind = entry.kind;
                } else {
                    kind = world._gapFillPending ? 'gapfill' : 'backfill';
                }

                if (kind === 'gapfill') {
                    // These lines are NEWER than what we have (an after_seq request), so
                    // they're appended and deduped - handled entirely separately from the
                    // phase 1/2 backfill prepend logic below (a gap-fill is normally tiny and
                    // finishes in one or two requests).
                    const wasBottom = isAtBottom();
                    const container = elements.outputContainer;
                    const oldScrollHeight = container.scrollHeight;
                    let appended = false;
                    let droppedCount = 0;

                    // Accept any line whose exact seq we have not already been delivered
                    // (PROTOCOL-ROADMAP.md Phase C). This branch used to require the line to
                    // be either NEWER than world._max_seq or to overlap a *recorded* gap,
                    // and dropped everything else - which meant a repair aimed at a hole the
                    // client never noticed opening (the poisoned-high-water-mark case that
                    // ResyncRequired and the server-side ack audit exist to fix) was itself
                    // discarded on arrival. Exact membership has no such blind spot: a
                    // genuine duplicate is still skipped, and everything else lands.
                    (msg.lines || []).forEach((line) => {
                        if (line.seq !== undefined && hasSeenSeq(world, line.seq)) {
                            droppedCount++;
                            return;
                        }
                        // Tail vs. mid-buffer. Insertion position for the mid-buffer case is
                        // best-effort (this handler doesn't consistently tag entries with
                        // _has_real_seq the way ServerData does) but never worse than the
                        // silent drop it replaces.
                        if (line.seq === undefined || line.seq > maxSeenSeq(world)) {
                            world.output_lines.push(line);
                        } else {
                            insertLinesBySeq(world, [line]);
                        }
                        if (line.seq !== undefined) {
                            markSeqRangeSeen(world, line.seq, line.seq);
                            world._max_seq = maxSeenSeq(world);
                        }
                        appended = true;
                    });
                    if (droppedCount > 0) {
                        console.warn('ScrollbackLines gap-fill reply: skipped lines already delivered', {
                            world_index: msg.world_index, skipped: droppedCount, msg_seq_range: (msg.lines || []).map(l => l.seq)
                        });
                    }

                    // Termination guard (PROTOCOL-ROADMAP.md Phase F). A reply that delivered
                    // nothing new means the seqs we are actually missing were not in it - the
                    // server has no line for them. Since requestGapFill() re-anchors on the
                    // (unmoved) contiguousFrontier(), retrying asks the identical question and
                    // gets the identical answer, forever. Count consecutive no-progress
                    // replies and, at the limit, declare the oldest hole lost so the frontier
                    // can move; see closeOldestSeqHole().
                    //
                    // A reply the server clamped against this world's pending backlog is NOT
                    // evidence of an unfillable hole: it delivered nothing because nothing was
                    // deliverable YET. Counting it tripped this guard, which clears
                    // _gapFillPending - the exact flag the PendingReleased re-drive is gated
                    // on - so the catch-up the clamp exists to defer never resumed.
                    const GAPFILL_MAX_NO_PROGRESS = 2;
                    world._gapFillNoProgress = (appended || msg.clamped_by_pending)
                        ? 0 : (world._gapFillNoProgress || 0) + 1;
                    if (world._gapFillNoProgress >= GAPFILL_MAX_NO_PROGRESS) {
                        const hole = oldestSeqHole(world);
                        world._gapFillNoProgress = 0;
                        if (hole && closeOldestSeqHole(world)) {
                            console.warn('Gap-fill could not recover a seq range - giving up on it', {
                                world_index: msg.world_index, hole: hole
                            });
                            send({
                                type: 'ReportGap',
                                world_index: msg.world_index,
                                hole_start: hole.start,
                                hole_end: hole.end,
                                attempts: GAPFILL_MAX_NO_PROGRESS,
                                source: window.Android ? 'android' : 'web'
                            });
                        } else {
                            // No hole to close (the frontier is already at our high-water
                            // mark) - there is nothing left to ask for, so stop the pump
                            // rather than re-requesting into the void.
                            world._gapFillPending = false;
                            updateScrollbackProgress();
                            setTimeout(function() { backfillNextWorld(); }, BACKFILL_DELAY_MS);
                            break;
                        }
                    }

                    if (appended && msg.world_index === currentWorldIndex) {
                        if (!wasBottom || grepRegex) {
                            renderOutput();
                            const newScrollHeight = container.scrollHeight;
                            container.scrollTop += (newScrollHeight - oldScrollHeight);
                        } else {
                            // Render NOW, not via scheduleCurrentWorldRepaint(). A gap-fill
                            // APPENDS the newest lines at the tail - the exact thing an
                            // at-the-bottom viewer is looking at - so there is nothing to
                            // defer. This branch used to borrow the backfill branch's
                            // "at the bottom => old content above the fold, no rush"
                            // reasoning below, which is only true for a PREPEND. Combined
                            // with the debounce having had no ceiling, that is what left
                            // the phone showing stale output for seconds after reconnect
                            // while the data was already in world.output_lines.
                            // renderOutput() ends in scrollToBottom(), so the viewer stays
                            // pinned - same end state as the debounced path, just at once.
                            renderOutput();
                        }
                    }
                    if (appended) scheduleWorldCacheSave(msg.world_index);

                    if (msg.backfill_complete) {
                        world._gapFillPending = false;
                    } else if (msg.clamped_by_pending) {
                        // The server withheld the rest behind this world's unreleased
                        // more-mode backlog. Asking again returns the identical answer until
                        // that backlog releases, so stop the loop - but stay ARMED
                        // (_gapFillPending left true) so the PendingReleased handler re-drives
                        // it, which is the contract handle_request_scrollback documents.
                        // Safe to hold indefinitely now that the scrollback badge no longer
                        // keys off this flag (it pinned the badge at 90% until the user
                        // happened to page through the backlog).
                    } else {
                        // Gap is bigger than one chunk - keep pulling from the new
                        // high-water mark until the daemon says we're caught up.
                        requestGapFill(msg.world_index);
                    }
                } else {
                    // Response to RequestScrollback. Two shapes land here:
                    //  - 'backfill'     : a before_seq request, i.e. OLDER history -> prepend
                    //  - 'initial-fill' : a before_seq:null request, which the server answers
                    //                     with the NEWEST N visible lines -> insert in seq
                    //                     order and mark as delivered
                    if (msg.lines && msg.lines.length > 0) {
                        const wasBottom = isAtBottom();
                        const container = elements.outputContainer;
                        const oldScrollHeight = container.scrollHeight;

                        if (kind === 'initial-fill') {
                            // Newest lines, not older history. Prepending them would bury the
                            // freshest output above whatever the world already holds (splash
                            // text, system lines, a stale cached tail) so it never appears at
                            // the bottom - one of the "missing output at the bottom" causes.
                            // Skip anything already delivered, then place each line where its
                            // seq says it belongs.
                            for (const line of msg.lines) {
                                if (line.seq !== undefined && hasSeenSeq(world, line.seq)) continue;
                                if (line.seq === undefined || line.seq > maxSeenSeq(world)) {
                                    world.output_lines.push(line);
                                } else {
                                    insertLinesBySeq(world, [line]);
                                }
                                if (line.seq !== undefined) markSeqRangeSeen(world, line.seq, line.seq);
                            }
                            world._max_seq = maxSeenSeq(world);
                        } else {
                            // Prepend received lines (they are older than what we have)
                            world.output_lines = msg.lines.concat(world.output_lines);

                            // Deliberately NOT marked into _seenRanges. This branch is the
                            // downward-growing deep-history region (a before_seq request); the
                            // resume/ack contract only ever covers the forward stream, and
                            // folding a not-yet-adjacent older chunk in here would make
                            // ranges[0] that chunk and drag contiguousFrontier() backwards.
                            // Pre-Phase-C code excluded backfill from _max_seq/_seqGaps for the
                            // same reason. Harmless for dedup: a gap-fill only ever asks for
                            // seqs above the frontier, so it can't re-deliver these.
                        }

                        // Update oldest seq for next backfill request
                        let minSeq = Infinity;
                        for (const line of msg.lines) {
                            if (line.seq !== undefined && line.seq < minSeq) minSeq = line.seq;
                        }
                        if (minSeq !== Infinity) world._oldest_seq = minSeq;

                        if (msg.world_index === currentWorldIndex) {
                            if (kind === 'initial-fill') {
                                // Content was added at the BOTTOM, not above, so the
                                // scrollTop correction below (which compensates for height
                                // inserted above the viewport) must not run - it would scroll
                                // away from the very lines we just added. Just repaint; if the
                                // user was at the bottom, renderOutput keeps them there.
                                renderOutput();
                            } else if (!wasBottom || grepRegex) {
                                // Scrolled up into history, or grep mode: the user needs to
                                // see the new content immediately, so render synchronously
                                // and correct scrollTop for the height added above.
                                renderOutput();
                                const newScrollHeight = container.scrollHeight;
                                container.scrollTop += (newScrollHeight - oldScrollHeight);
                            } else {
                                // At the bottom: the backfilled lines are old content added
                                // above the fold, not currently visible, so there's no rush
                                // to paint this exact chunk - but skipping the repaint
                                // entirely (as this used to do whenever the world already had
                                // a full screen) left the current/initial world's history
                                // unreachable by scrolling until something else forced a
                                // renderOutput() (e.g. a world switch). Coalesce instead: a
                                // fast backfill can deliver many chunks in quick succession,
                                // and rebuilding the DOM on every single one would restart
                                // CSS animations (e.g. blink) on whatever's currently visible.
                                scheduleCurrentWorldRepaint();
                            }
                        }
                        scheduleWorldCacheSave(msg.world_index);
                    }
                    // Continue or finish backfill.
                    // Phase 1 is one chunk per world - always advance to the next
                    // world in the queue regardless of backfill_complete.
                    // Phase 2 is round-robin - re-queue this world at the back if it
                    // still needs more (and history isn't exhausted), then advance
                    // to whichever world is now at the front (may be a different one).
                    if (msg.backfill_complete) {
                        world._backfill_exhausted = true;
                    }
                    if (backfillPhase === 2) {
                        const received = visibleLineCount(world);
                        if (received < backfillTotalTarget && !world._backfill_exhausted) {
                            backfillWorldQueue.push(msg.world_index);
                        }
                    }
                }

                // Shared tail (PROTOCOL-ROADMAP.md's seq-drift fix): both branches must
                // advance the pump, not just the backfill one - the old bare `break` in the
                // gap-fill branch is exactly what let backfillInProgress get stuck forever
                // whenever a reply was misrouted there.
                updateScrollbackProgress();
                setTimeout(function() {
                    backfillNextWorld();
                }, backfillPhase === 1 ? 0 : BACKFILL_DELAY_MS);
                break;
            }

            case 'ServerReloading':
                reloadReconnect = true;
                reloadReconnectAttempts = 0;
                break;

            default:
                console.log('Unknown message type:', msg.type);
        }
    }

    // Handle incoming line with more-mode logic
    function handleIncomingLine(text, ts, worldIndex, lineIndex, markedNew, fromServer = true, highlightColor = null) {
        if (text === undefined || text === null) return;

        const visibleLines = getVisibleLineCount();
        const threshold = Math.max(1, visibleLines - 2);

        if (paused) {
            // Already paused, queue the line info
            pendingLines.push({ text, ts, worldIndex, lineIndex, markedNew: markedNew || false, fromServer, highlightColor });
            updateStatusBar();
        } else if (moreModeEnabled && linesSincePause >= threshold) {
            // Trigger pause
            paused = true;
            pendingLines.push({ text, ts, worldIndex, lineIndex, markedNew: markedNew || false, fromServer, highlightColor });
            // Scroll to bottom to show what we have so far
            scrollToBottom();
            updateStatusBar();
        } else {
            // Normal display - append the line
            linesSincePause++;
            appendNewLine(text, ts, worldIndex, lineIndex, markedNew, fromServer, highlightColor);
        }
    }

    // How many lines are held back for the current world, local queue plus the server's.
    function pendingTotal() {
        const world = worlds[currentWorldIndex];
        return pendingLines.length + (world ? (world.pending_count || 0) : 0);
    }

    // Release `count` lines of pending output. `count` is a VISUAL ROW budget on the wire
    // (WsMessage::ReleasePending, see World::release_pending) - a line wrapping to three rows
    // costs three - which is what makes it the right unit for the drag gesture, whose input is
    // also measured in screen rows. `count: 0` means "everything"; use releaseAll() for that.
    function releaseLines(count) {
        const world = worlds[currentWorldIndex];
        const serverPending = world ? (world.pending_count || 0) : 0;

        // Check if there's anything to release (local or server)
        if (pendingLines.length === 0 && serverPending === 0) return;

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

    // Release one screenful of pending lines
    function releaseScreenful() {
        releaseLines(Math.max(1, getVisibleLineCount() - 2));
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
            appendNewLine(item.text, item.ts, item.worldIndex, item.lineIndex, item.markedNew, item.fromServer, item.highlightColor);
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

    // The scrollback-download budget (Remote Lines) must be spent only on VISIBLE
    // (non-gagged) lines, never on gagged ones - they're invisible unless F2/show_tags is on,
    // so counting them against the budget means the client can finish "downloaded" while
    // showing far fewer visible lines than Remote Lines implies (including this indicator's
    // own percentage). Recomputing a full filter over output_lines on every check is fine at
    // this scale (client-side output_lines is hundreds to low thousands of lines, checked on
    // backfill-pump ticks, not per rendered frame) - no incremental counter needed.
    function visibleLineCount(world) {
        if (!world.output_lines) return 0;
        let count = 0;
        for (const line of world.output_lines) {
            if (!line.gagged) count++;
        }
        return count;
    }

    // Total VISIBLE lines available server-side for a world - prefers the
    // server-authoritative total_visible_lines field (added alongside this fix) over
    // total_output_lines (raw, gagged included), which the client can't turn into a visible
    // count on its own since it has no knowledge of gagged status for lines it hasn't
    // downloaded yet. Falls back to total_output_lines only against an older server that
    // predates total_visible_lines (see websocket.rs's field doc comment) - an
    // overcount there, but strictly better than treating "no data" as zero.
    function totalVisibleLines(world) {
        if (world.total_visible_lines !== undefined && world.total_visible_lines !== null) {
            return world.total_visible_lines;
        }
        return world.total_output_lines || 0;
    }

    // Start backfill after InitialState is processed.
    // Phase 1: give every under-filled world a screenful, current world first,
    // one request per world, no waiting for full history.
    // Phase 2 (started automatically once phase 1 drains): round-robin the
    // remaining worlds in 200-line chunks until each hits backfillTotalTarget
    // lines or the server reports its history is exhausted.
    function startBackfill() {
        backfillInProgress = false;
        backfillWorldQueue = [];
        backfillCurrentWorld = null;
        backfillPhase = 1;
        backfillPhase1Target = Math.max(75, getVisibleLineCount());
        backfillTotalTarget = Math.max(remoteInitialLines || 100, backfillPhase1Target);

        // A world hydrated from a local buffer (in-memory reconnect, or the
        // persistent cache on a cold start - see the InitialState handler's
        // _hydratedFromLocal flag) doesn't need a backfill at all: it already has
        // its history, it just needs to catch up on whatever accumulated on the
        // server since we last had it. Gap-fill it directly instead of queuing.
        // A world that's NOT hydrated from local (fresh connect, no cache) always
        // goes through the normal queue below, even if InitialState front-loaded
        // it with real lines - those are the newest lines, not a local buffer to
        // extend, so requesting an "everything since" gap-fill for it would be a
        // wasted round trip (the server has nothing newer to send).
        //
        // PROTOCOL-ROADMAP.md Step 5: skip the explicit requestGapFill() call for a
        // world the server is already resuming unprompted (_resumedFromServer, set in
        // the InitialState handler from the AuthRequest.resume we sent on connect) -
        // asking again here would just be a redundant round trip for the exact same
        // range. Only the cache-hydrated case (no resume coverage possible - see
        // _resumedFromServer's comment above) still needs this client-driven request.
        worlds.forEach((world, idx) => {
            // hasDeliveredSeqs(), not `world._max_seq` truthiness: seq 0 is a legitimate
            // real value (World::next_seq starts at 0), so a world whose only delivered line
            // is seq 0 used to fail this guard, skip the gap-fill, and fall through to the
            // older-history queues only - never fetching anything newer.
            if (world._hydratedFromLocal && hasDeliveredSeqs(world) && !world._resumedFromServer) {
                requestGapFill(idx);
            }
        });

        // Build the phase 1 queue: current world first, then others. Only
        // worlds still short of a screenful belong in phase 1 - a world that
        // already has >= backfillPhase1Target lines locally (but still has
        // more total history) skips straight to phase 2 instead of getting a
        // wasteful near-empty request here. A world with total > received
        // always has something worth fetching, even when _oldest_seq is null
        // (e.g. build_initial_state's aggregate line budget ran out before
        // reaching it, even though the world has real history server-side) -
        // requestBackfillChunk sends before_seq: null in that case, which the
        // daemon already handles as "send the last N lines". Do not skip these
        // worlds here, or they stay permanently empty. Hydrated-from-local worlds
        // are excluded here too (handled above) unless they're still short of a
        // screenful even after being seeded locally, in which case they still
        // want older history on top of the gap-fill.
        // Worlds with an outstanding gap-fill are excluded here too (in addition to the
        // _resumedFromServer skip above) - a world can reach this point with
        // _gapFillPending already true from the requestGapFill() call in the loop just
        // above, or from a cache-hydrated world's own gap-fill request queued at
        // InitialState time. Queuing it into phase 1 as well would race the two request
        // kinds against each other for the same undifferentiated world, which is exactly
        // the ambiguity the request_id correlator (see the ScrollbackLines handler) exists
        // to resolve - excluding it here is cheaper and avoids the race outright.
        const queue = [];
        worlds.forEach((world, idx) => {
            const total = totalVisibleLines(world);
            const received = visibleLineCount(world);
            if (total > received && received < backfillPhase1Target && !world._gapFillPending) {
                if (idx === currentWorldIndex) {
                    queue.unshift(idx);
                } else {
                    queue.push(idx);
                }
            }
        });

        backfillWorldQueue = queue;
        backfillInProgress = true;
        updateScrollbackProgress();
        // Delay before first request to let UI settle
        setTimeout(function() {
            if (backfillWorldQueue.length === 0) {
                startBackfillPhase2();
            } else {
                backfillNextWorld();
            }
        }, 500);
    }

    // Move to the next world in the backfill queue (phase 1), or start/continue
    // the round-robin phase 2 queue once phase 1 has drained.
    function backfillNextWorld() {
        if (backfillWorldQueue.length === 0) {
            if (backfillPhase === 1) {
                startBackfillPhase2();
                return;
            }
            backfillInProgress = false;
            backfillCurrentWorld = null;
            updateScrollbackProgress();
            return;
        }
        backfillCurrentWorld = backfillWorldQueue.shift();
        requestBackfillChunk(backfillCurrentWorld);
    }

    // Build the phase 2 round-robin queue: every world still short of
    // backfillTotalTarget (and not already known to be exhausted), current
    // world first. Each cycle through the queue sends one chunk per world.
    function startBackfillPhase2() {
        backfillPhase = 2;
        const queue = [];
        worlds.forEach((world, idx) => {
            const received = visibleLineCount(world);
            // !world._gapFillPending: same race-avoidance as the phase 1 queue builder
            // above - don't queue an ordinary backfill chunk request for a world that
            // already has a gap-fill outstanding.
            if (received < backfillTotalTarget && !world._backfill_exhausted && !world._gapFillPending) {
                if (idx === currentWorldIndex) {
                    queue.unshift(idx);
                } else {
                    queue.push(idx);
                }
            }
        });
        if (queue.length === 0) {
            backfillInProgress = false;
            backfillCurrentWorld = null;
            updateScrollbackProgress();
            return;
        }
        backfillWorldQueue = queue;
        backfillNextWorld();
    }

    // Aggregate scrollback-download indicator for the status bar - shows while
    // backfillInProgress is true (spans both phases, see startBackfill/
    // startBackfillPhase2/backfillNextWorld above) AND there's an actual gap left
    // to fetch. Global across all worlds, not just the current one, so it doesn't
    // jump around as you switch worlds mid-backfill.
    // Percentage is floored to a multiple of 10, so it can only ever read "100%"
    // at true completion - which means the moment the numbers reach that point,
    // there's nothing left to communicate and the badge should already be gone,
    // not still sitting on screen. Hiding is therefore driven directly by
    // totalReceived reaching totalGoal, not by backfillInProgress alone: the
    // phase-transition state machine (startBackfill's 500ms settle delay, the
    // backfillNextWorld()/startBackfillPhase2() empty-queue checks) can take a
    // while longer to formally flip that flag to false, and this indicator
    // shouldn't visibly linger for that bookkeeping to catch up - it was most
    // noticeable right after a reconnect where a world is already fully
    // hydrated from memory/cache: the ratio reads 100% immediately, so it must
    // hide immediately too.
    //
    // This badge tracks the scrollback-DEPTH download and nothing else. It briefly also
    // stayed up while a reconnect gap-fill was outstanding, on the reasoning that depth
    // alone isn't completion - but that predicate was far too broad. It counted every entry
    // in pendingScrollbackRequests, which holds ordinary 'backfill'/'initial-fill' chunks
    // and not just 'gapfill', so it was true for essentially the whole download. The
    // visible result: the ratio-met hide was suppressed and the badge sat on its last
    // drawable number - 90% - after the download had genuinely finished.
    //
    // The reason it was made to linger is also gone: it was compensating for a reconnect
    // showing stale output, and that was fixed directly in the same change (gap-fill now
    // renders on arrival, and the repaint debounce has a max-wait ceiling). So the badge
    // is back to reporting exactly one thing, honestly: 0/10/.../90 while the depth
    // download runs, hidden the moment it completes.
    function updateScrollbackProgress() {
        if (!elements.statusScrollback) return;
        if (!backfillInProgress) {
            elements.statusScrollback.style.display = 'none';
            return;
        }
        let totalReceived = 0;
        let totalGoal = 0;
        worlds.forEach((world) => {
            const goal = Math.min(backfillTotalTarget, totalVisibleLines(world));
            const received = visibleLineCount(world);
            totalGoal += goal;
            totalReceived += Math.min(received, goal);
        });
        if (totalGoal <= 0 || totalReceived >= totalGoal) {
            elements.statusScrollback.style.display = 'none';
            return;
        }
        const pct = Math.floor((totalReceived / totalGoal) * 100 / 10) * 10;
        elements.statusScrollback.style.display = '';
        elements.statusScrollbackPct.textContent = pct + '%';
    }

    // Send a RequestScrollback for the given world. Phase 1 asks for just enough
    // to reach a screenful; phase 2 asks for a round-robin chunk, clamped so the
    // final chunk for a world never overshoots backfillTotalTarget.
    function requestBackfillChunk(worldIndex) {
        const world = worlds[worldIndex];
        if (!world) {
            // World no longer exists (e.g. removed since being queued), skip it.
            backfillNextWorld();
            return;
        }
        const received = visibleLineCount(world);
        // count now means "N visible lines" server-side (handle_request_scrollback), so
        // sizing it from the visible received count keeps this request from over/under
        // asking relative to what's actually still needed to reach the target.
        const count = backfillPhase === 1
            ? Math.max(1, backfillPhase1Target - received)
            : Math.max(1, Math.min(BACKFILL_PHASE2_CHUNK_SIZE, backfillTotalTarget - received));
        // before_seq may legitimately be null here (a world that received zero lines in
        // InitialState despite having real history) - the server handles that as "send the
        // last N VISIBLE lines" (handle_request_scrollback's third branch), i.e. it returns
        // the NEWEST lines, not older history.
        //
        // That difference matters for how the reply is applied, so tag the kind HERE rather
        // than re-deriving it from _oldest_seq when the reply lands (by then it may have
        // changed). A newest-lines reply must be inserted in seq order and marked into
        // _seenRanges; blind-prepending it - which is correct for a genuine before_seq
        // request - buries the newest output above whatever the world already holds and
        // leaves it unmarked. That was a second, independent cause of "missing output at
        // the bottom".
        const isInitialFill = world._oldest_seq === null || world._oldest_seq === undefined;
        const requestId = registerScrollbackRequest(worldIndex, isInitialFill ? 'initial-fill' : 'backfill');
        send({
            type: 'RequestScrollback',
            world_index: worldIndex,
            count: count,
            before_seq: world._oldest_seq,
            request_id: requestId
        });
    }

    // ============================================================================
    // Reconnect gap-fill and bounded scrollback cache
    // ============================================================================
    // Two complementary mechanisms avoid re-downloading the whole scrollback on
    // reconnect:
    //  - In-memory: the InitialState handler no longer wipes worlds[] on a resync
    //    (see the `isResync`/`worlds.length > 0` check there) - it merges by name
    //    and keeps each world's existing output_lines/_max_seq. This alone covers the
    //    common case on Android, where backgrounding/resuming/network changes
    //    reconnect the WebSocket but never destroy the WebView's JS heap.
    //  - Persistent (IndexedDB): a bounded per-world cache, capped at
    //    remoteInitialLines lines, so a cold start / full page reload / process
    //    death can also gap-fill instead of doing a full backfill. Capped so it
    //    never grows unbounded no matter how long a world has been open.
    // Both funnel through requestGapFill(), which issues a RequestScrollback with
    // after_seq (the newest seq we already have - added alongside the existing
    // before_seq direction, see websocket.rs) instead of a full backfill. Because
    // per-world seq numbers are monotonic and persisted across daemon restarts
    // and /flush (see RequestScrollback in daemon.rs/main.rs), an after_seq
    // request is always correct - there's no discontinuity case to guard against.

    const WORLD_CACHE_DB_NAME = 'clay-scrollback-cache';
    const WORLD_CACHE_STORE = 'worlds';
    let worldCacheDbPromise = null;
    let worldCacheServerId = null;
    // Populated best-effort by preloadWorldCacheForServer(), which is called from
    // init() - well before InitialState typically arrives. A world not in here by
    // the time InitialState is processed just falls back to a normal backfill;
    // this is an optimization, never a correctness dependency.
    let worldCacheLoaded = {};

    // Identify "which server" for the cache key. On Android, the WebSocket may
    // connect to a different local port on every SSH-tunnel restart (the tunnel
    // uses a random ephemeral local port), so the actually-connected host:port is
    // not a stable key. The user-configured remote host (SSH target, or the
    // direct host when not tunneling) is stable across those restarts and is
    // what "the same server" means to the user. Non-Android clients have no such
    // indirection, so the page's own origin is already stable.
    function getServerIdentity() {
        try {
            if (window.Android && typeof window.Android.getConnectionInfo === 'function') {
                const info = JSON.parse(window.Android.getConnectionInfo());
                const host = info.remoteHost || info.localHost || 'unknown';
                return host + ':' + (info.port || '');
            }
        } catch (e) { /* fall through to origin-based identity */ }
        return (typeof location !== 'undefined' && location.host) ? location.host : 'local';
    }

    // Last-active-world persistence: a synchronous localStorage read/write (unlike
    // the IndexedDB scrollback cache above, this needs to resolve immediately when
    // InitialState is processed, with no async preload/race to manage) so a cold
    // start (page reload, app/process restart) can restore the world the user was
    // actually looking at instead of defaulting to the server's current_world_index
    // - see the InitialState handler. Scoped per-server via getServerIdentity(),
    // same as the scrollback cache, so switching servers doesn't restore the wrong
    // world. localStorage is already relied on transitively in this WebView (the
    // scrollback cache's IndexedDB requires the same "DOM storage enabled" setting),
    // so this doesn't introduce a new capability dependency - but every call is
    // still wrapped in try/catch since storage can be unavailable or full and that
    // must never break the app.
    let lastPersistedWorldName = null;
    function lastWorldStorageKey() {
        return 'clay_last_world_' + getServerIdentity();
    }
    function persistLastActiveWorld() {
        const world = worlds[currentWorldIndex];
        const name = (world && world.name) || null;
        if (name === lastPersistedWorldName) return;
        lastPersistedWorldName = name;
        try {
            if (name) {
                localStorage.setItem(lastWorldStorageKey(), name);
            } else {
                localStorage.removeItem(lastWorldStorageKey());
            }
        } catch (e) { /* storage unavailable/full - non-critical, ignore */ }
    }

    function openWorldCacheDb() {
        if (worldCacheDbPromise) return worldCacheDbPromise;
        worldCacheDbPromise = new Promise((resolve) => {
            if (typeof indexedDB === 'undefined') { resolve(null); return; }
            try {
                const req = indexedDB.open(WORLD_CACHE_DB_NAME, 1);
                req.onupgradeneeded = function() {
                    const db = req.result;
                    if (!db.objectStoreNames.contains(WORLD_CACHE_STORE)) {
                        db.createObjectStore(WORLD_CACHE_STORE);
                    }
                };
                req.onsuccess = function() { resolve(req.result); };
                req.onerror = function() { resolve(null); };
            } catch (e) { resolve(null); }
        });
        return worldCacheDbPromise;
    }

    function worldCacheKey(serverId, worldName) {
        return serverId + '|' + worldName;
    }

    // Load every cached world for this server in one pass (world names aren't
    // known yet at connect time - InitialState hasn't arrived). Call this as
    // early as possible so the read has time to finish before InitialState is
    // processed; it's best-effort and never blocks anything.
    function preloadWorldCacheForServer(serverId) {
        worldCacheServerId = serverId;
        worldCacheLoaded = {};
        openWorldCacheDb().then((db) => {
            if (!db) return;
            try {
                const tx = db.transaction(WORLD_CACHE_STORE, 'readonly');
                const store = tx.objectStore(WORLD_CACHE_STORE);
                const prefix = serverId + '|';
                const range = IDBKeyRange.bound(prefix, prefix + '￿');
                const req = store.openCursor(range);
                req.onsuccess = function() {
                    const cursor = req.result;
                    if (!cursor) return;
                    const worldName = String(cursor.primaryKey).slice(prefix.length);
                    if (cursor.value && cursor.value.lines && cursor.value.lines.length > 0) {
                        worldCacheLoaded[worldName] = cursor.value;
                    }
                    cursor.continue();
                };
            } catch (e) { /* ignore - cache is best-effort */ }
        });
    }

    // Debounced, capped save of a world's tail to the cache. Capped at
    // remoteInitialLines (the same setting that bounds the backfill target) so
    // the cache can never grow past what a fresh connect would download anyway.
    let worldCacheSaveTimers = {};
    const WORLD_CACHE_SAVE_DEBOUNCE_MS = 2000;
    // Delete a world's persistent scrollback cache entry outright - used when the
    // InitialState handler detects the entry is stale relative to the server's current
    // session (see the server-restart-detection guard there) so a reconnect within the same
    // stale window doesn't hydrate from the same poisoned buffer again before fresh data
    // naturally overwrites it via scheduleWorldCacheSave.
    function clearWorldCacheEntry(worldName) {
        if (!worldName || !worldCacheServerId) return;
        openWorldCacheDb().then((db) => {
            if (!db) return;
            try {
                const tx = db.transaction(WORLD_CACHE_STORE, 'readwrite');
                tx.objectStore(WORLD_CACHE_STORE).delete(worldCacheKey(worldCacheServerId, worldName));
            } catch (e) { /* ignore */ }
        });
    }

    function scheduleWorldCacheSave(worldIndex) {
        const world = worlds[worldIndex];
        if (!world || !world.name || !worldCacheServerId) return;
        const name = world.name;
        if (worldCacheSaveTimers[name]) clearTimeout(worldCacheSaveTimers[name]);
        worldCacheSaveTimers[name] = setTimeout(function() {
            delete worldCacheSaveTimers[name];
            const w = worlds[worldIndex];
            if (!w || w.name !== name) return; // world list changed under us
            const cap = Math.max(10, remoteInitialLines || 100);
            const lines = (w.output_lines || []).slice(-cap);
            const maxSeq = w._max_seq || 0;
            // Persist the delivered-seq record too (PROTOCOL-ROADMAP.md Phase C) - without
            // this, a hole known mid-session is silently forgotten across a full reload
            // (cold-start hydrate from this cache), making contiguousFrontier() wrongly
            // report _max_seq as if the hole had never existed. `lines` is capped, so the
            // ranges routinely cover more seqs than the persisted slice does; that's the
            // point (rebuildSeenRanges unions the two rather than recomputing from lines).
            // maxSeq is still written for the benefit of an older client reading this
            // entry after a downgrade, and is what the pre-Phase-C shape keyed on.
            const seenRanges = w._seenRanges || [];
            // Stamped so the next hydration can tell whether these seqs still refer to
            // a live sequence space - see the epoch check in the InitialState handler.
            const seqEpoch = w._seq_epoch || 0;
            openWorldCacheDb().then((db) => {
                if (!db) return;
                try {
                    const tx = db.transaction(WORLD_CACHE_STORE, 'readwrite');
                    tx.objectStore(WORLD_CACHE_STORE).put({ lines: lines, maxSeq: maxSeq, seenRanges: seenRanges, seqEpoch: seqEpoch }, worldCacheKey(worldCacheServerId, name));
                } catch (e) { /* ignore */ }
            });
        }, WORLD_CACHE_SAVE_DEBOUNCE_MS);
    }

    // Issue a gap-fill request for a world that already has a buffer (from memory or
    // the persistent cache), asking only for lines newer than `fromSeq` (defaults to
    // its highest known seq - the reconnect/continuation case). Falls back to a normal
    // backfill if no anchor is available at all. `fromSeq` is passed explicitly by the
    // ResyncRequired handler (PROTOCOL-ROADMAP.md Step 5), which has a server-supplied
    // seq to resync from that may differ from our own (possibly stale) world._max_seq -
    // 0 is a legitimate explicit value there (client hasn't acked anything yet for that
    // world), so it's distinguished from "no argument" rather than treated as falsy.
    function requestGapFill(worldIndex, fromSeq) {
        const world = worlds[worldIndex];
        if (!world) return;
        const hasExplicitFromSeq = (fromSeq !== undefined && fromSeq !== null);
        // contiguousFrontier(), NOT world._max_seq (PROTOCOL-ROADMAP.md Phase C). Asking
        // from the high-water mark meant a hole BELOW it was never actually requested - the
        // client-driven repair path could only ever fetch what came after the damage, never
        // the damage itself. An explicit fromSeq (the ResyncRequired path, where the server
        // names the range it owes us) still wins.
        const seq = hasExplicitFromSeq ? fromSeq : contiguousFrontier(world);
        // `!seq` would also reject a legitimate frontier of 0 and silently downgrade to an
        // older-history-only backfill. Fall back only when we genuinely hold no delivered
        // seqs for this world.
        if (!hasExplicitFromSeq && !hasDeliveredSeqs(world)) {
            queueNormalBackfill(worldIndex);
            return;
        }
        world._gapFillPending = true;
        // request_id: 0 is RESERVED for the server-initiated unprompted resume replay (see
        // pendingScrollbackRequests' doc comment) - this is a client-initiated request, so
        // it always gets a real allocated id, never 0.
        const requestId = registerScrollbackRequest(worldIndex, 'gapfill');
        send({
            type: 'RequestScrollback',
            world_index: worldIndex,
            count: BACKFILL_PHASE2_CHUNK_SIZE,
            after_seq: seq,
            request_id: requestId
        });
    }

    // Fold a single world into the normal backfill queue (used as a fallback
    // when gap-fill isn't applicable). Safe to call whether or not a backfill
    // pass is already under way for other worlds.
    function queueNormalBackfill(worldIndex) {
        if (!backfillWorldQueue.includes(worldIndex)) {
            backfillWorldQueue.push(worldIndex);
        }
        if (!backfillInProgress) {
            backfillInProgress = true;
            updateScrollbackProgress();
            backfillNextWorld();
        }
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
            request_key: false,
            resume: buildResumeAckListForAuthRequest(), resume_epochs: buildResumeEpochList(), client_version: clientVersion(),
            client_uid: clientUid
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

        // Store for silent re-auth on reconnect/hot-reload (browser clients only —
        // Android uses window.Android.savePassword, WebView uses AUTO_PASSWORD).
        if (!window.Android && !window.AUTO_PASSWORD) {
            lastGoodPassword = password;
            lastGoodUsername = username;
        }

        // Hash password with SHA-256, then apply challenge-response
        hashPassword(password).then(async hash => {
            // Challenge-response: SHA256(SHA256(password) + challenge)
            const challengeHash = serverChallenge ? await hashPassword(hash + serverChallenge) : hash;
            const msg = { type: 'AuthRequest', password_hash: challengeHash, request_key: false, challenge_response: !!serverChallenge, resume: buildResumeAckListForAuthRequest(), resume_epochs: buildResumeEpochList(), client_version: clientVersion(), client_uid: clientUid };
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
            const msg = { type: 'AuthRequest', password_hash: challengeHash, request_key: false, challenge_response: !!serverChallenge, resume: buildResumeAckListForAuthRequest(), resume_epochs: buildResumeEpochList(), client_version: clientVersion(), client_uid: clientUid };
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

        // Only release held output / reset more-mode state when following live output at
        // the bottom. If the user has scrolled up to read history, keep their position —
        // the command still sends and its reply queues below, revealed on scroll-down.
        if (isAtBottom()) {
            if (paused) {
                releaseAll();
            }
            // Reset lines since pause counter on user input
            linesSincePause = 0;
        }

        // Clear splash on user input (server will also clear and send WorldFlushed)
        if (worlds[currentWorldIndex] && worlds[currentWorldIndex].showing_splash) {
            worlds[currentWorldIndex].showing_splash = false;
            renderOutput();
        }

        const cmdTrimmed = cmd.trim();

        // Intercept /window --grep locally only — this opens a client-side filtered
        // view that the server has no concept of (parse_command's Command::Window
        // only knows a plain world-name argument, so a --grep string would be
        // mistaken for one server-side). Plain "/window"/"/window <world>" is NOT
        // intercepted below: it's sent to the server so Command::Window's
        // auto-connect-if-disconnected behavior (main.rs) applies, same as the TUI;
        // the server replies with OpenWindow, which the handler above then opens.
        if (cmdTrimmed === '/window' || cmdTrimmed.startsWith('/window ')) {
            var winArgs = cmdTrimmed.length > 8 ? cmdTrimmed.substring(8).trim() : '';

            // Check for --grep flag: /window --grep pattern [-w world] [--regexp]
            // Supports quoted patterns: --grep "*some pattern*" or --grep pattern
            var grepMatch = winArgs.match(/--grep\s+"([^"]+)"/) || winArgs.match(/--grep\s+'([^']+)'/) || winArgs.match(/--grep\s+(\S+)/);
            if (grepMatch) {
                elements.input.value = '';
                var grepPattern = grepMatch[1];
                var grepWorldMatch = winArgs.match(/-w\s+(\S+)/);
                var grepWorld = grepWorldMatch ? grepWorldMatch[1] : null;
                var grepRegexp = winArgs.includes('--regexp');

                if (window.WEBVIEW_MODE) {
                    sendIpc('grep-window:' + JSON.stringify({
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
                    var grepUrl = winProto + '://' + winHost + ':' + winPort + basePath() + '/'
                        + '?grep=' + encodeURIComponent(grepPattern)
                        + (grepWorld ? '&world=' + encodeURIComponent(grepWorld) : '')
                        + (grepRegexp ? '&regexp=1' : '');
                    window.open(grepUrl, '_blank');
                }
                return;
            }
            // Not --grep: fall through to the default server-send path below.
        }

        // Intercept /reload in remote WebView mode — restart the local GUI binary
        // (master WebView passes through to server which handles exec_reload with state)
        if (window.WEBVIEW_MODE && !window.AUTO_PASSWORD &&
            (cmdTrimmed === '/reload' || cmdTrimmed.startsWith('/reload '))) {
            elements.input.value = '';
            sendIpc('reload');
            return;
        }

        // Intercept /connect in remote WebView mode — this client attaches to/detaches
        // from remote Clay servers directly; never forwarded to the currently-attached
        // server (the master WebView's /connect is handled server-side instead).
        if (window.WEBVIEW_MODE && !window.AUTO_PASSWORD &&
            (cmdTrimmed === '/connect' || cmdTrimmed.startsWith('/connect '))) {
            elements.input.value = '';
            var connectArgs = cmdTrimmed.length > 9 ? cmdTrimmed.substring(9).trim().split(/\s+/).filter(Boolean) : [];
            if (connectArgs.length === 0) {
                appendClientLine('Usage: /connect host:port  (or)  /connect host port  (or)  /connect --close');
            } else if (connectArgs[0] === '--close') {
                pendingRemoteConnect = null;
                sendIpc('connect-close');
            } else if (connectArgs[0] === '--cancel') {
                if (pendingRemoteConnect) {
                    pendingRemoteConnect = null;
                    appendClientLine('Cancelled.');
                } else {
                    appendClientLine('No pending /connect to cancel.');
                }
            } else {
                var connectAddr = connectArgs.length > 1 ? (connectArgs[0] + ':' + connectArgs[1]) : connectArgs[0];
                var now = Date.now();
                if (pendingRemoteConnect && pendingRemoteConnect.addr === connectAddr &&
                    (now - pendingRemoteConnect.requestedAt) < REMOTE_CONNECT_CONFIRM_WINDOW_MS) {
                    pendingRemoteConnect = null;
                    sendIpc('connect:' + connectAddr);
                } else {
                    pendingRemoteConnect = { addr: connectAddr, requestedAt: now };
                    appendClientLine('This will disconnect from the current server and attach to ' +
                        connectAddr + ' instead. Run /connect ' + connectAddr +
                        ' again within 15s to confirm, or /connect --cancel.');
                }
            }
            return;
        }

        // Intercept /import — the password/auth-key must be collected client-side (never
        // sent as a bounced command line) and delivered via a dedicated ImportSettings
        // message instead. See plan i-d-like-to-make-snuggly-rain.md.
        if (cmdTrimmed === '/import' || cmdTrimmed.startsWith('/import ')) {
            elements.input.value = '';
            var importArgs = cmdTrimmed.length > 7 ? cmdTrimmed.substring(7).trim().split(/\s+/).filter(Boolean) : [];
            var importAddr = importArgs.length > 1 ? (importArgs[0] + ':' + importArgs[1]) : (importArgs[0] || '');
            showImportDialog(importAddr);
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

    // Restore the current world's MUD prompt to the prompt display area.
    // sendCommand() clears elements.prompt on every submit; this restores it
    // after internal commands that don't cause the server to send a new prompt.
    function redisplayCurrentPrompt() {
        const world = worlds[currentWorldIndex];
        if (!world || !world.prompt) return;
        elements.prompt.innerHTML = sanitizeHtml(parseAnsi(world.prompt));
    }

    // Execute a command locally (called from server via ExecuteLocalCommand message).
    // The server's parse_command() is the single source of truth for command parsing.
    // This function only handles the UI/popup side of commands.
    function executeLocalCommand(cmd) {
        const trimmed = cmd.trim();
        const parts = trimmed.split(/\s+/);
        const firstWord = parts[0].toLowerCase();
        const args = parts.slice(1);
        let promptAfter = true; // redisplay prompt after most local commands

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
                ws.send(JSON.stringify({ type: 'RequestConnectionsList' }));
                promptAfter = false; // prompt shown after ConnectionsListResponse instead
                break;

            case '/worlds':
            case '/world':
                if (args.length === 0) {
                    openWorldSelectorPopup();
                } else if (args[0] === '-e') {
                    // /worlds -e [name] - open world editor. If the name doesn't match an
                    // existing world, create it (same CreateWorld round trip addNewWorld() uses)
                    // and open the editor on the real new world once WorldCreated arrives —
                    // mirrors the TUI's find_or_create_world behavior.
                    const name = args.length > 1 ? args.slice(1).join(' ') : null;
                    if (name) {
                        const idx = worlds.findIndex(w => w.name.toLowerCase() === name.toLowerCase());
                        if (idx >= 0) {
                            openWorldEditorPopup(idx);
                        } else {
                            send({ type: 'CreateWorld', name: name });
                        }
                    } else {
                        openWorldEditorPopup(currentWorldIndex);
                    }
                } else if (args[0] === '-l') {
                    // /worlds -l <name> - server already connected, just switch local view.
                    // Unreachable-if-not-found in practice: -l parses to Command::WorldSwitch
                    // server-side, which is handled entirely there (own "not found" ServerData
                    // message, main.rs) and never bounced here via ExecuteLocalCommand.
                    if (args.length > 1) {
                        const name = args.slice(1).join(' ');
                        const idx = worlds.findIndex(w => w.name.toLowerCase() === name.toLowerCase());
                        if (idx >= 0) switchWorldLocal(idx);
                    }
                } else {
                    // /worlds <name> - server already connected if needed, switch local view.
                    // Same as -l above: the not-found case is handled server-side, not here.
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
                if (args.length > 0) {
                    appendClientLine("This form of /note is console-only. Plain /note opens the current world's notes.", currentWorldIndex, 'system');
                    break;
                }
                openNoteEditor();
                break;

            case '/quit':
                // Close the window — use IPC for WebView GUI, window.close() for browser
                if (window.WEBVIEW_MODE) {
                    sendIpc('quit');
                } else {
                    window.close();
                }
                break;

            case '/reload':
                // Hot reload — local only, never restart the remote server
                if (window.WEBVIEW_MODE) {
                    sendIpc('reload');
                } else {
                    window.location.reload();
                }
                break;

            case '/update':
                // Update the local client binary
                if (window.WEBVIEW_MODE) {
                    // WebView GUI: delegate to native side via IPC
                    const forceFlag = args.length > 0 && (args[0] === '-f' || args[0] === '--force');
                    sendIpc(forceFlag ? 'update-force' : 'update');
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
                promptAfter = false; // server sends next prompt naturally
                break;
        }

        if (promptAfter) redisplayCurrentPrompt();
    }

    // Reset the render window for the world being left. ▶ markers are NOT cleared here: the
    // server owns them (OutputLine::display_id) and sends us a ReleasedNew once the
    // MarkWorldSeen this triggers reaches it. Clearing them locally would be a second,
    // driftable copy of that state. Shared by switchWorldLocal and the WorldSwitchResult
    // handler so both world-switch paths reset it - previously only switchWorldLocal did.
    function clearLeavingWorldState(oldIndex) {
        const oldWorld = worlds[oldIndex];
        // Reset the render window on the world we're leaving too, so returning to it
        // later starts fresh at RENDER_WINDOW_INITIAL rather than wherever a previous
        // deep-scroll session left it - keeps per-world DOM cost bounded across
        // multiple world visits, not just within one.
        if (oldWorld) oldWorld._renderWindow = RENDER_WINDOW_INITIAL;
    }

    // Switch world locally (does not affect console)
    function switchWorldLocal(index) {
        if (lockedWorld) return; // Don't switch worlds in locked windows
        if (index >= 0 && index < worlds.length && index !== currentWorldIndex) {
            mcmpStopAll();
            const previousWorldIndex = currentWorldIndex;
            // Clear new line indicators on the world we're LEAVING (matches console behavior)
            clearLeavingWorldState(previousWorldIndex);
            currentWorldIndex = index;
            // Clear splash on world switch, but only for a world that has actually
            // connected before - a fresh/never-connected world's output_lines is
            // just the splash art itself (see the InitialState handler's identical
            // was_connected check above for why this guard is required).
            const newWorld = worlds[index];
            if (newWorld && newWorld.showing_splash && newWorld.was_connected && newWorld.output_lines && newWorld.output_lines.length > 0) {
                newWorld.showing_splash = false;
            }
            // Reset more-mode state for new world
            paused = false;
            pendingLines = [];
            linesSincePause = 0;
            // Take ▶ ownership before the first paint, not after the MarkWorldSeen below
            // round-trips - otherwise this render shows the text bare and the markers appear
            // a moment later. The ClaimedNew that MarkWorldSeen triggers reconciles it.
            claimUnviewedLocally(index);
            renderOutput();
            updateStatusBar();
            // Update prompt to show new world's prompt
            const world = worlds[currentWorldIndex];
            if (world && world.prompt) {
                elements.prompt.innerHTML = sanitizeHtml(parseAnsi(world.prompt));
            } else {
                elements.prompt.textContent = '';
            }
            // Notify server that this world has been seen (syncs unseen count). Tell it
            // which world we're leaving (previous_world_index) so it can clear that world's
            // marked_new indicators server-side even across a reconnect, when the server's
            // own per-client "current world" tracking (keyed by an ephemeral client id) has
            // been lost - see MarkWorldSeen's doc comment in websocket.rs.
            send({ type: 'MarkWorldSeen', world_index: index, previous_world_index: previousWorldIndex });
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

    // Search popup functions (F5)
    function openSearchPopup() {
        searchPopupOpen = true;
        searchText = '';
        searchMatchIndices = [];
        searchCurrentPos = -1;
        elements.searchPopup.style.display = 'block';
        elements.searchInput.value = '';
        if (elements.searchMatchInfo) elements.searchMatchInfo.textContent = '';
        elements.searchInput.focus();
    }

    function closeSearchPopup() {
        searchPopupOpen = false;
        searchText = '';
        searchMatchIndices = [];
        searchCurrentPos = -1;
        elements.searchPopup.style.display = 'none';
        elements.input.focus();
        renderOutput();
    }

    function computeSearchMatches() {
        const world = worlds[currentWorldIndex];
        if (!world) { searchMatchIndices = []; return; }
        const lines = world.output_lines || [];
        if (!searchText) { searchMatchIndices = []; return; }
        searchMatchIndices = [];
        for (let i = 0; i < lines.length; i++) {
            const lineObj = lines[i];
            if (!lineObj) continue;
            const rawLine = typeof lineObj === 'string' ? lineObj : lineObj.text;
            const plain = stripAnsiForFilter(String(rawLine).replace(/[\r\n]+/g, ''));
            if (matchesFilter(plain, searchText)) {
                searchMatchIndices.push(i);
            }
        }
        // Start at most recent match
        searchCurrentPos = searchMatchIndices.length > 0 ? searchMatchIndices.length - 1 : -1;
    }

    function updateSearchMatchInfo() {
        if (!elements.searchMatchInfo) return;
        if (!searchText) {
            elements.searchMatchInfo.textContent = '';
        } else if (searchMatchIndices.length === 0) {
            elements.searchMatchInfo.textContent = '(no matches)';
        } else {
            elements.searchMatchInfo.textContent = '(' + (searchCurrentPos + 1) + '/' + searchMatchIndices.length + ')';
        }
    }

    function updateSearch() {
        searchText = elements.searchInput.value;
        computeSearchMatches();
        updateSearchMatchInfo();
        renderOutput();
    }

    function advanceSearch() {
        if (searchMatchIndices.length === 0) return;
        if (searchCurrentPos > 0) {
            searchCurrentPos--;
        } else {
            searchCurrentPos = searchMatchIndices.length - 1;
        }
        updateSearchMatchInfo();
        renderOutput();
    }

    // Help popup functions (/help)
    // Full command/keybinding reference, structured as sections of {l, r} rows with
    // occasional {heading} markers for sub-groups. This is no longer the primary Help
    // view (see buildHelpQuickStartCards below) - renderHelpReferenceHtml() turns it
    // into the collapsed "full reference" disclosure instead.
    const helpSections = [
        { heading: 'Commands', note: '(/help &lt;command&gt; for details)', rows: [
            { heading: 'Connection' },
            { l: '/worlds', r: 'Open world selector' },
            { l: '/worlds &lt;name&gt;', r: 'Connect to or create world' },
            { l: '/worlds -e [name]', r: 'Edit world settings' },
            { l: '/worlds -l &lt;name&gt;', r: 'Connect without auto-login' },
            { l: '/worlds -b &lt;name&gt;', r: 'Connect in background (no switch)' },
            { l: '/addworld [-x] name host port', r: 'Create a new world' },
            { l: '/disconnect (or /dc)', r: 'Disconnect from server' },
            { l: '/connections (or /l)', r: 'List connected worlds' },
            { l: '/connect &lt;host[:port]&gt;', r: 'Attach to a remote Clay server' },
            { l: '/connect --close', r: 'Detach and become an independent master' },
            { l: '/connect --cancel', r: 'Cancel a pending confirmation' },
            { heading: 'Communication' },
            { l: '/send [-W] [-w&lt;world&gt;] [-n] &lt;text&gt;', r: 'Send text to world(s)' },
            { l: '', r: '-W=all worlds, -n=no newline' },
            { l: '/notify &lt;message&gt;', r: 'Send notification to mobile' },
            { l: '/say &lt;text&gt;', r: 'Speak text aloud (TTS)' },
            { heading: 'Lookup &amp; Translation' },
            { l: '/dict &lt;word&gt;', r: 'Look up word definition' },
            { l: '/urban &lt;word&gt;', r: 'Look up Urban Dictionary' },
            { l: '/translate &lt;lang&gt; &lt;text&gt;', r: 'Translate text (or /tr)' },
            { l: '/url &lt;url&gt;', r: 'Shorten a URL' },
            { heading: 'Actions &amp; Triggers' },
            { l: '/actions [world]', r: 'Open actions editor' },
            { l: '/gag [pattern]', r: 'List gags, or gag lines matching pattern' },
            { l: '/&lt;action_name&gt; [args]', r: 'Execute named action' },
            { heading: 'Settings' },
            { l: '/setup', r: 'Open global settings' },
            { l: '/web', r: 'Open web/WebSocket settings' },
            { l: '/tag', r: 'Toggle MUD tag display (F2)' },
            { l: '/font', r: 'Font settings (web/GUI)' },
            { heading: 'Display' },
            { l: '/menu', r: 'Open menu popup' },
            { l: '/flush', r: 'Clear output buffer' },
            { l: '/dump', r: 'Dump scrollback to file' },
            { l: '/note [file]', r: 'Open split-screen editor' },
            { l: '/window [world]', r: 'Open new browser window/tab' },
            { l: '/window --grep [pattern]', r: 'Open grep/filter window' },
            { heading: 'System' },
            { l: '/help [topic]', r: 'Show help (topic = command)' },
            { l: '/version', r: 'Show version info' },
            { l: '/reload', r: 'Hot reload binary' },
            { l: '/update [-f]', r: 'Download and install update' },
            { l: '/testmusic', r: 'Test ANSI music playback' },
            { l: '/quit', r: 'Exit client' },
            { heading: 'Security' },
            { l: '/ban', r: 'Show banned hosts' },
            { l: '/unban &lt;host&gt;', r: 'Remove host from ban list' },
            { l: '/remote', r: 'List connected WebSocket clients' },
            { l: '/remote --kill &lt;id&gt;', r: 'Disconnect a client' },
            { l: '/remote --pause &lt;id&gt;', r: 'Toggle pause on a client' },
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
            { l: 'F5', r: 'Search history' },
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

    // Switch the CURRENT page/WebView into note-editing mode in place: hides the
    // normal chat chrome and shows the note editor view, same DOM swap the
    // page-load-time noteMode branch in the InitialState handler does for a
    // dedicated note window/tab. This is the only option on Android — its
    // single WebView has no multi-window support (no onCreateWindow /
    // setSupportMultipleWindows wired up in MainActivity.java), so window.open()
    // there either crashes (file:// URI exposure in local/standalone mode) or
    // silently hijacks the one WebView with no way back (remote mode); see
    // exitNoteMode() for the reverse. Desktop/plain-web keep the real
    // separate-window/tab behavior below since they actually support it.
    function enterNoteMode(worldIndex) {
        noteMode = { world_index: worldIndex };
        if (elements.statusBar) elements.statusBar.style.display = 'none';
        if (elements.inputContainer) elements.inputContainer.style.display = 'none';
        if (elements.navBar) elements.navBar.style.display = 'none';
        if (elements.outputContainer) elements.outputContainer.style.display = 'none';
        if (elements.tabsRibbon) elements.tabsRibbon.style.display = 'none';
        if (elements.iconBar) elements.iconBar.style.display = 'none';
        if (elements.noteEditorView) elements.noteEditorView.style.display = 'flex';
        send({ type: 'RequestNoteEditorState', world_index: worldIndex });
    }

    // Reverses enterNoteMode(): restore the normal chat view. Mirrors the
    // hide/restore pattern showAuthModal(false) already uses for the same set
    // of containers.
    function exitNoteMode() {
        noteMode = null;
        if (elements.noteEditorView) elements.noteEditorView.style.display = 'none';
        if (elements.statusBar) elements.statusBar.style.display = '';
        if (elements.inputContainer) elements.inputContainer.style.display = '';
        if (elements.navBar) elements.navBar.style.display = '';
        if (elements.outputContainer) elements.outputContainer.style.display = '';
        document.title = 'Clay MUD Client';
        setupToolbars(deviceMode);
        renderOutput();
        updateStatusBar();
        elements.input.focus();
    }

    // Opens the current world's notes. Desktop GUI/plain-web get a genuine
    // separate window (native OS window via webview-GUI's IPC, a new browser
    // tab otherwise) — not an in-page modal, not a shell-out. Android instead
    // switches the current page into note mode in place (see enterNoteMode()
    // for why: its WebView has no multi-window support). Shared by the /note
    // command and the status-bar note icon (both should behave identically).
    // Only the no-args "current world" form is supported here; `/note -l` and
    // `/note <file>` remain console-only (see plan doc for why).
    function openNoteEditor() {
        if (!worlds[currentWorldIndex]) return;
        if (window.Android) {
            enterNoteMode(currentWorldIndex);
        } else if (window.WEBVIEW_MODE) {
            var notePayload = { world_index: currentWorldIndex, world_name: worlds[currentWorldIndex].name };
            sendIpc('note-window:' + JSON.stringify(notePayload));
        } else {
            var noteUrl = window.location.origin + basePath() + '/?note=' + currentWorldIndex;
            window.open(noteUrl, '_blank');
        }
    }

    // Task-oriented quick-start cards for the web/GUI/Android Help popup - "tap X to do
    // Y", never /command syntax, since this audience taps menus rather than typing
    // commands (unlike console, which keeps its own separate, terser getting-started
    // help - see src/popup/definitions/help.rs; deliberately different content, not a
    // CLAUDE.md parity gap). Icons are inline stroke-style SVGs (24x24 viewBox,
    // stroke-width 1.6) so they render crisp at any zoom without an icon font.
    // "Make it yours" gets its editor links filled in at render time (needs baseUrl).
    function buildHelpQuickStartCards(baseUrl) {
        return [
            {
                icon: '<path d="M9 2v4M15 2v4M6 6h12v5a6 6 0 0 1-12 0V6z"/><path d="M12 17v5"/>',
                title: 'Connect to a world',
                desc: 'Tap the world name, or open <strong>World Selector</strong> and hit <strong>Add</strong> to set one up.'
            },
            {
                icon: '<path d="M4 8h13l-3-3M20 16H7l3 3"/>',
                title: 'Switch between worlds',
                desc: 'Tap the world name, swipe the tabs, or use <kbd>&#9650;</kbd>/<kbd>&#9660;</kbd> at the bottom.'
            },
            {
                icon: '<path d="M3 12l18-8-8 18-2-8-8-2z"/>',
                title: 'Type &amp; send',
                desc: 'Type below and tap <strong>Send</strong>. Hold <kbd>&#9650;</kbd>/<kbd>&#9660;</kbd> to reuse something you typed before.'
            },
            {
                icon: '<path d="M4 8l8-4 8 4-8 4-8-4z"/><path d="M4 13l8 4 8-4"/>',
                title: 'Catch up on output',
                desc: '<strong>MORE</strong> means text is waiting, tap to reveal it. <strong>ACT</strong> flags other busy worlds.'
            },
            {
                icon: '<path d="M13 2 4 14h6l-1 8 9-12h-6l1-8z"/>',
                title: 'Automate with Actions',
                desc: '<strong>Actions</strong> auto-respond to things the world sends. Pin one to the toolbar as a button.'
            },
            {
                icon: '<path d="M4 6h6M14 6h6M4 12h10M18 12h2M4 18h13M20 18h1"/><circle cx="12" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="19" cy="18" r="2"/>',
                title: 'Make it yours',
                desc: 'Settings links to the <a href="' + baseUrl + '/theme-editor" target="_blank">Theme</a> and ' +
                    '<a href="' + baseUrl + '/keybind-editor" target="_blank">Keybind</a> editors. The A&nbsp;&mdash;&nbsp;A slider resizes text.'
            },
            {
                icon: '<path d="M2 8.5a16 16 0 0 1 20 0M5.5 12a11 11 0 0 1 13 0M9 15.5a6 6 0 0 1 6 0"/><circle cx="12" cy="19" r="1.2" fill="currentColor" stroke="none"/>',
                title: 'Use it from anywhere',
                desc: 'Turn on remote access in <strong>Settings &rarr; Web</strong>, and manage this device&rsquo;s key there.'
            },
            {
                icon: '<rect x="4.5" y="4" width="12" height="16" rx="1.5"/><path d="M8 9.5h5M8 13h5"/><path d="M15.5 15l4-4 2 2-4 4h-2v-2z"/>',
                title: 'Keep notes',
                desc: 'The notepad icon (when a world has notes) opens a simple editor for it.'
            }
        ];
    }

    // Transforms helpSections (the exhaustive command/keybinding list, unchanged) into
    // one <details> per category for the collapsed "full reference" section - same
    // content the old flat table had, single source of truth, just not the first thing
    // shown anymore. Commands' rows contain inline {heading} markers for sub-groups
    // (Connection, Communication, ...); other top-level sections become one group each.
    function renderHelpRefGroup(label, rows) {
        let html = '<details class="help-ref-group"><summary>' + label +
            '<svg class="help-ref-chev" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg></summary>';
        html += '<table class="help-ref-table">';
        for (const row of rows) {
            html += '<tr><td>' + row.l + '</td><td>' + row.r + '</td></tr>';
        }
        html += '</table></details>';
        return html;
    }

    function renderHelpReferenceHtml() {
        let html = '';
        for (const section of helpSections) {
            if (section.heading === 'Commands') {
                let label = null;
                let rows = [];
                const flush = () => {
                    if (label && rows.length) html += renderHelpRefGroup(label, rows);
                    rows = [];
                };
                for (const row of section.rows) {
                    if (row.heading) {
                        flush();
                        label = row.heading;
                    } else {
                        rows.push(row);
                    }
                }
                flush();
            } else {
                // "Display" is reused for two different groups (a Commands sub-heading,
                // and this top-level function-key list) - rename the keybinding one so
                // the collapsed group labels aren't ambiguous duplicates.
                const label = section.heading === 'Display' ? 'Function Keys' : section.heading;
                html += renderHelpRefGroup(label, section.rows);
            }
        }
        return html;
    }

    // This client's own bundled version, or null when it genuinely isn't known.
    //
    // `window.CLIENT_VERSION` is injected three different ways: substituted into index.html
    // server-side for the browser (http.rs) and the desktop WebView (webview_gui.rs), but set
    // at runtime on Android (MainActivity.buildVarInjectionScript, from the APK's versionName)
    // because the page is loaded straight off `file:///android_asset/` where nothing rewrites
    // the template. So the raw `{{CLIENT_VERSION}}` placeholder is a real possible value here
    // - it is what the bundled asset literally contains before Android's onPageFinished
    // injection runs, and it has escaped once before (see the InitialState version-mismatch
    // check, which reuses this). Callers get null and show something sensible rather than
    // printing braces at the user.
    function clientVersion() {
        const v = window.CLIENT_VERSION;
        if (typeof v !== 'string' || v.length === 0 || v.indexOf('{{') !== -1) return null;
        return v;
    }

    function openHelpPopup() {
        helpPopupOpen = true;
        const baseUrl = getBaseUrl();
        // Name the build in the title bar - on web/GUI/Android this is the only place the
        // client's own version is visible (the console has /version and its startup banner).
        if (elements.helpTitle) {
            const v = clientVersion();
            elements.helpTitle.textContent = v ? 'Help for Clay v' + v : 'Help';
        }

        let html = '<div class="help-quickstart">';
        for (const card of buildHelpQuickStartCards(baseUrl)) {
            html += '<div class="help-card">' +
                '<span class="help-card-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">' + card.icon + '</svg></span>' +
                '<div><h3>' + card.title + '</h3><p>' + card.desc + '</p></div>' +
                '</div>';
        }
        html += '</div>';

        html += '<details class="help-ref-toggle"><summary>Show full command &amp; keybinding reference' +
            '<svg class="help-ref-chev" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg></summary>' +
            '<div class="help-ref-body">' + renderHelpReferenceHtml() + '</div></details>';

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
            'Keyboard Visible: Force the on-screen keyboard to',
            '  stay up on phones/tablets. Off lets the OS decide.',
            '  Always ignored (keyboard hidden) when a hardware',
            '  keyboard is attached.', '',
            'Debug: Enables debug logging to ~/.clay/debug.log.', '',
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
            'Startup: Run command when Clay starts/hot-reloads.', '',
            'GUI Menu Shortcut: Show this action as a one-click',
            '  shortcut tile in the web/GUI icon bar.'
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

    // Wildcard-to-regex for action triggers — mirrors actions.rs::wildcard_to_regex.
    // Unlike filterWildcardToRegex (unanchored "contains" semantics for /filter),
    // action patterns must match the entire line, so this anchors with ^...$.
    function actionWildcardToRegex(pattern) {
        let regex = '^';
        let i = 0;
        while (i < pattern.length) {
            const c = pattern[i];
            if (c === '\\') {
                const next = i + 1 < pattern.length ? pattern[i + 1] : undefined;
                if (next === '*' || next === '?' || next === '\\') {
                    regex += '\\' + next;
                    i += 2;
                } else {
                    // Lone backslash (incl. trailing) - escape it for regex
                    regex += '\\\\';
                    i += 1;
                }
                continue;
            }
            if (c === '*') {
                regex += '(.*)';
            } else if (c === '?') {
                regex += '(.)';
            } else if (c === '"' || c === '“' || c === '”') {
                regex += '["“”]';
            } else if (c === '\'' || c === '‘' || c === '’') {
                regex += '[\'‘’]';
            } else if ('.+^$|()[]{}'.includes(c)) {
                regex += '\\' + c;
            } else {
                regex += c;
            }
            i++;
        }
        regex += '$';
        return regex;
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
            // Check world match: mirrors actions.rs::action_matches_world — an
            // action.world that's empty (or all-comma/whitespace) is global and
            // matches every world; otherwise it's a comma-separated list, each
            // segment trimmed and compared case-insensitively.
            if (action.world && !action.world.split(',').every(w => w.trim() === '') &&
                !action.world.split(',').some(w => w.trim().toLowerCase() === worldName.toLowerCase())) continue;
            // Action-level match type (legacy per-pattern type ignored)
            const matchType = action.match_type || 'Regexp';
            // Build effective pattern list (new multi-pattern or legacy single)
            const pats = Array.isArray(action.patterns) && action.patterns.length > 0
                ? action.patterns
                : (action.pattern ? [{ pattern: action.pattern }] : []);
            for (const mp of pats) {
                const patText = typeof mp === 'string' ? mp : (mp.pattern || '');
                if (!patText || patText.trim() === '') continue;
                try {
                    let pat = patText;
                    if (matchType === 'Wildcard') {
                        pat = actionWildcardToRegex(patText);
                    }
                    const regex = new RegExp(pat, 'i');
                    if (regex.test(plainLine)) return true;
                } catch (e) {
                    // Invalid regex, skip
                }
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

    // Display-time marker for client-generated lines. Mirrors
    // rendering.rs::process_output_line (src/rendering.rs:400-406): the prefix
    // is added AFTER the visually-empty early-return and BEFORE stripMudTag(),
    // and is never stored. world.output_lines[].text, the IndexedDB cache,
    // grep/filter matching, and the /quote backtick re-capture all stay
    // prefix-free, exactly as OutputLine.text does on the Rust side.
    const CLIENT_LINE_PREFIX = '✨ ';
    function applyClientPrefix(text, fromServer) {
        if (fromServer !== false) return text; // default true, like the Rust flag
        if (!text || stripAnsiForFilter(text).trim() === '') return text; // is_visually_empty()
        return CLIENT_LINE_PREFIX + text;
    }

    // Expandable DOM render window (scrollback-reachability fix, PROTOCOL-ROADMAP.md
    // follow-on): the DOM used to hard-cap at the newest 500 lines regardless of how much
    // history a world actually held, with a stale comment claiming PageUp re-rendered to
    // reveal more (it only ever moved scrollTop - handlePgUp does not call renderOutput()).
    // The window now grows in RENDER_WINDOW_STEP increments as the user scrolls toward the
    // top (see the outputContainer.onscroll handler), up to RENDER_WINDOW_MAX - matching
    // remote_initial_lines' own upper clamp, so a --gui/Android client can reach everything
    // it's permitted to hold locally. The ceiling is a deliberate performance bound for
    // WebKitGTK/Android WebView (tested-safe, not arbitrary) - lower RENDER_WINDOW_MAX if a
    // low-end device struggles, don't remove the mechanism.
    const RENDER_WINDOW_INITIAL = 500;
    const RENDER_WINDOW_STEP = 500;
    const RENDER_WINDOW_MAX = 5000;
    // How close to the top (px) triggers growing the window - large enough to grow before
    // the user actually hits the physical top and sees a hard stop.
    const RENDER_WINDOW_GROW_TRIGGER_PX = 300;

    // rAF-throttled: outputContainer.onscroll can fire many times per animation frame, but
    // growing the window triggers a full renderOutput() rebuild, which is not something to
    // do more than once per frame.
    let renderWindowCheckScheduled = false;
    function scheduleRenderWindowCheck() {
        if (renderWindowCheckScheduled) return;
        renderWindowCheckScheduled = true;
        requestAnimationFrame(function() {
            renderWindowCheckScheduled = false;
            const world = worlds[currentWorldIndex];
            if (!world) return;
            const container = elements.outputContainer;
            const totalHeld = world.output_lines ? world.output_lines.length : 0;
            const currentWindow = world._renderWindow || RENDER_WINDOW_INITIAL;
            const ceiling = Math.min(RENDER_WINDOW_MAX, totalHeld);
            if (container.scrollTop < RENDER_WINDOW_GROW_TRIGGER_PX && currentWindow < ceiling) {
                world._renderWindow = Math.min(currentWindow + RENDER_WINDOW_STEP, ceiling);
                renderOutput({ preserveScroll: true });
            } else if (isAtBottom() && currentWindow !== RENDER_WINDOW_INITIAL) {
                // Back at the bottom: reset so the DOM shrinks back down on the next full
                // render (world switch away and back, resync, etc.) rather than staying
                // elevated indefinitely after a single deep scroll-back. Not itself a reason
                // to force a rebuild right now - the user is reading live output at the
                // bottom, and rebuilding would only disrupt that for no benefit.
                world._renderWindow = RENDER_WINDOW_INITIAL;
            }
        });
    }

    // Whether a line renders with the ▶ new-text indicator FOR THIS CLIENT.
    //
    // Ownership is recorded per line (`display_id`, see OutputLine::display_id in main.rs)
    // and assigned server-side when a client displays a world, so this is a plain equality
    // test against our own id. Every rule about *when* a line becomes new lives in the
    // server's claim/release logic, not here.
    //
    // Replaces a seq-window test against a per-WORLD watermark pair
    // (new_from_seq/viewed_from_seq). One shared pair per world cannot express "new for you
    // but not for me": with two instances on different worlds it suppressed ▶ for everyone
    // whenever any one client viewed a world, and one client leaving a world wiped another
    // client's markers. `myDisplayId` comes from InitialState.your_display_id and is stable
    // across reconnects (it is derived from our own client_uid), which is what preserves our
    // markers through a brief transport drop.
    function lineIsNew(lineObj, world) {
        if (!lineObj) return false;
        if (!myDisplayId) return false; // no id yet, or an older server: never claim a marker
        return lineObj.display_id === myDisplayId;
    }

    // Take ▶ ownership of this world's unviewed lines locally, ahead of the server saying so.
    //
    // Ownership is server-authoritative, but the server can only tell us after a round trip -
    // we render the world the instant the user switches to it, then MarkWorldSeen goes out,
    // then ClaimedNew comes back. That is one full network round-trip during which the text is
    // already on screen *without* its ▶ markers, so they visibly pop in a moment later. On a
    // phone over mobile data it reads as the app redrawing itself.
    //
    // So predict it. This mirrors World::claim_unviewed exactly - same three conditions, same
    // order - and the ClaimedNew that follows reconciles any difference (see its handler).
    // A wrong guess is possible only when our `viewed` copy is stale, which needs a second
    // remote client to have claimed the same world since our last update; it self-corrects on
    // the next message rather than persisting.
    function claimUnviewedLocally(worldIndex) {
        const w = worlds[worldIndex];
        if (!w || !myDisplayId || !Array.isArray(w.output_lines)) return;
        const guessed = new Set();
        for (const line of w.output_lines) {
            if (line.viewed) continue;
            line.viewed = true;
            // from_server defaults true on the wire; archive lines never take a marker.
            if (line.from_server !== false && !line.from_archive) {
                line.display_id = myDisplayId;
                guessed.add(line.seq);
            }
        }
        // Remembered so the reconciling ClaimedNew can tell "we guessed this" apart from
        // "we already owned this", and only revoke the former. Timestamped because a guess is
        // only meaningful until the reply that answers it: an outstanding guess that is never
        // answered must not survive to be revoked by some unrelated later ClaimedNew, which is
        // exactly how ▶ used to appear and then vanish a moment later.
        w._optimisticClaim = guessed.size ? { seqs: guessed, at: Date.now() } : null;
    }

    function renderOutput(opts) {
        const preserveScroll = !!(opts && opts.preserveScroll);
        const world = worlds[currentWorldIndex];

        // If no world selected (multiuser mode before connecting), show splash
        if (!world) {
            if (splashLines && splashLines.length > 0) {
                renderSplashScreen();
            }
            return;
        }

        // WebView mode (desktop GUI) or the Android app: show the image splash instead of
        // the ASCII-art one. clay2.png always lives alongside index.html — desktop's
        // clay://localhost/ custom protocol (webview_gui.rs) and Android's bundled
        // file:///android_asset/web/ (copyLogoAsset in build.gradle) both serve it from the
        // same directory as the page itself, so a relative src resolves correctly under
        // either origin (also handles Windows WebView2, which serves "clay://" as
        // "http://clay.localhost/"). Do NOT use window.location.origin here — Android loads
        // index.html via file:///android_asset/..., so origin is "file://" and an
        // origin-absolute src would resolve to a nonexistent path on the device filesystem.
        if ((window.WEBVIEW_MODE || typeof Android !== 'undefined') && world.showing_splash) {
            elements.output.innerHTML = '<div style="display:flex;flex-direction:column;align-items:center;justify-content:center;height:100%;gap:5px;">' +
                '<img src="clay2.png" alt="Clay" style="width:200px;height:200px;">' +
                '<div style="color:#ff87ff;font-style:italic;">A 90dies mud client written today</div>' +
                '<div style="color:#888;">/help for how to use clay</div>' +
                '</div>';
            return;
        }

        const lines = world.output_lines || [];

        // When search popup is active and has a match, truncate output at the match line
        // so the matched line appears at the bottom of the output area
        let searchEndIdx = lines.length;
        if (searchPopupOpen && searchCurrentPos >= 0 && searchCurrentPos < searchMatchIndices.length) {
            searchEndIdx = searchMatchIndices[searchCurrentPos] + 1;
        }

        // Render window: starts at RENDER_WINDOW_INITIAL, grows toward RENDER_WINDOW_MAX as
        // the user scrolls up (see the outputContainer.onscroll handler below). searchEndIdx
        // above already anchors the window's END at a search match when one is active, same
        // as before this change - a match is always the newest line in the window regardless
        // of _renderWindow's current size.
        const renderWindow = world._renderWindow || RENDER_WINDOW_INITIAL;
        const startIdx = Math.max(0, searchEndIdx - renderWindow);

        // Build lines as HTML with explicit <br> line breaks
        const htmlParts = [];
        for (let i = startIdx; i < searchEndIdx; i++) {
            const lineObj = lines[i];
            if (lineObj === undefined || lineObj === null) continue;

            // Handle both old string format and new object format
            const rawLine = typeof lineObj === 'string' ? lineObj : lineObj.text;
            const lineTs = typeof lineObj === 'object' ? lineObj.ts : null;
            const lineGagged = typeof lineObj === 'object' ? lineObj.gagged : false;
            const lineHighlightColor = typeof lineObj === 'object' ? lineObj.highlight_color : null;
            const lineMarkedNew = typeof lineObj === 'object' ? lineIsNew(lineObj, world) : false;
            const lineFromArchive = typeof lineObj === 'object' ? lineObj.from_archive : false;
            const lineFromServer = typeof lineObj === 'object' ? lineObj.from_server : true;

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

            const prefixedLine = applyClientPrefix(cleanLine, lineFromServer);
            const strippedText = showTags ? prefixedLine : stripMudTag(prefixedLine);
            const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
            // Skip Discord emoji conversion when showTags is enabled so users can see original text
            const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
            const newLinePrefix = (newLineIndicator && lineMarkedNew) ? '<span style="color:#00ff00;">▶</span> ' : '';
            const archivePrefix = lineFromArchive ? '🛢️ ' : '';
            let html = tsPrefix + newLinePrefix + archivePrefix + (showTags ? processed : convertDiscordEmojis(processed));

            // Apply /highlight color from action command (takes priority)
            if (lineHighlightColor !== null && lineHighlightColor !== undefined) {
                const bgColor = colorNameToCss(lineHighlightColor);
                html = `<span style="background-color: ${bgColor}; display: block;">${html}</span>`;
            }
            // Apply F8 action highlighting if enabled (and no explicit highlight color)
            else if (highlightActions && lineMatchesAction(cleanLine, world.name || '')) {
                html = `<span class="action-highlight">${html}</span>`;
            }

            htmlParts.push(`<span class="line" data-line-idx="${i}">${html}</span>`);
        }

        // Each line is its own block-level element (the "line" class, see style.css) so
        // it auto-stacks vertically without needing <br> separators — this is also what
        // makes the wrapspace hanging-indent CSS trick possible (text-indent only works
        // on elements that establish their own line-wrapping context, not plain inline
        // spans separated by <br>).
        // Defense-in-depth: strip any event handler attributes that slipped through
        // (e.g. from MUD-supplied text) before it ever reaches the DOM.
        if (preserveScroll) {
            // Growing the render window while scrolled up must not jump the view - capture
            // the height before rebuilding and restore scrollTop by the delta, the same
            // pattern the ScrollbackLines handler already uses for a prepend. Deliberately
            // does NOT call scrollToBottom() (that's the whole point of preserving position).
            const container = elements.outputContainer;
            const oldScrollHeight = container.scrollHeight;
            const oldScrollTop = container.scrollTop;
            elements.output.innerHTML = sanitizeHtml(htmlParts.join(''));
            const newScrollHeight = container.scrollHeight;
            container.scrollTop = oldScrollTop + (newScrollHeight - oldScrollHeight);
        } else {
            elements.output.innerHTML = sanitizeHtml(htmlParts.join(''));
            scrollToBottom();
        }

        // Clear unseen for current world
        world.unseen_lines = 0;
    }

    // Append a client-generated message to output
    // style: 'info' (✨ prefix, applied at display time by appendNewLine) or
    // 'system' (yellow color, ✨ still applies since these are from_server: false)
    function appendClientLine(text, worldIndex = currentWorldIndex, style = 'info') {
        const prefixes = {
            info: '',
            system: '\x1b[33m'
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
                appendNewLine(clientText, ts, worldIndex, lineIndex, false, false);
            }
        }
    }

    // Append a new line to current world's output (already visible)
    // `highlightColor` must be threaded through here, not just applied in renderOutput():
    // this is the path a line takes when it arrives live, and a /hilite'd line used to render
    // plain until something forced a full re-render (a world switch, a resync) - at which
    // point the same line suddenly gained its background. renderOutput() below is the
    // reference for what a line should look like; keep the two in step.
    function appendNewLine(text, ts, worldIndex, lineIndex, markedNew, fromServer = true, highlightColor = null) {
        // Strip newlines/carriage returns
        const cleanText = String(text).replace(/[\r\n]+/g, '');

        // Format timestamp prefix if showTags is enabled
        const tsPrefix = showTags && ts ? `<span class="timestamp">${formatTimestamp(ts)}</span>` : '';

        const prefixedText = applyClientPrefix(cleanText, fromServer);
        const strippedText = showTags ? prefixedText : stripMudTag(prefixedText);
        const displayText = showTags && tempConvertEnabled ? convertTemperatures(strippedText) : strippedText;
        // Skip Discord emoji conversion when showTags is enabled so users can see original text
        const processed = linkifyUrls(parseAnsi(insertWordBreaks(displayText)));
        const newLinePrefix = (newLineIndicator && markedNew) ? '<span style="color:#00ff00;">▶</span> ' : '';
        let html = tsPrefix + newLinePrefix + (showTags ? processed : convertDiscordEmojis(processed));
        if (highlightColor !== null && highlightColor !== undefined) {
            html = `<span style="background-color: ${colorNameToCss(highlightColor)}; display: block;">${html}</span>`;
        }

        // "line" is a block-level element (see style.css) so it auto-stacks below the
        // previous one — no <br> separator needed (also what makes the wrapspace
        // hanging-indent CSS work; see renderOutput()'s comment).
        // Defense-in-depth: strip any event handler attributes that slipped through
        // (e.g. from MUD-supplied text) before it ever reaches the DOM.
        elements.output.insertAdjacentHTML('beforeend', sanitizeHtml(`<span class="line" data-line-idx="${lineIndex}">${html}</span>`));

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

        // Read a single theme color CSS var (e.g. --theme-fg, --theme-bg) as RGB, with
        // a fallback if the var is missing/malformed. Mirrors getThemeAnsiPalette()'s
        // pattern of reading theme vars set by the server, just for a single color
        // instead of the 16-entry ANSI palette.
        function getThemeVarRgb(varName, fallbackRgb) {
            const val = getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
            if (val && val.startsWith('#') && val.length === 7) {
                return [parseInt(val.slice(1, 3), 16), parseInt(val.slice(3, 5), 16), parseInt(val.slice(5, 7), 16)];
            }
            return fallbackRgb;
        }

        // Resolves the theme's default background - needed for reverse-video (SGR 7):
        // "no explicit background" normally just lets the parent element's CSS
        // background show through, but reversing text requires a concrete color to
        // swap *in as* the new foreground, so the default has to be resolved here too.
        function getThemeBgRgb() {
            return getThemeVarRgb('--theme-bg', [19, 25, 38]); // #131926, Clay's dark theme default
        }

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
            return getThemeVarRgb('--theme-fg', [232, 228, 236]); // Default text color (matches theme fg)
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

        // SGR 7 (reverse video): swap foreground and background, resolving both to
        // concrete RGB first - a swap can land on an arbitrary theme color that has no
        // ansi-* class of its own (e.g. reversed default-colored text becomes a
        // background of the theme's fg color), so this can't be expressed by just
        // toggling classes the way bold/italic/underline are. Deliberately not a CSS
        // filter:invert() - that inverts each color channel independently rather than
        // swapping two colors, which is wrong whenever the background isn't pure black.
        // Non-color classes (bold/italic/underline/blink) pass through untouched.
        function applyReverseVideo(classes, fgStyle, bgStyle) {
            const fgRgb = getFgRgb(classes, fgStyle);
            const bgRgb = getBgRgb(classes, bgStyle) || getThemeBgRgb();
            const kept = classes.filter(c => c === 'ansi-bold' || c === 'ansi-italic' || c === 'ansi-underline' || c === 'ansi-blink');
            return {
                classes: kept,
                fgStyle: `color:rgb(${bgRgb[0]},${bgRgb[1]},${bgRgb[2]});`,
                bgStyle: `background-color:rgb(${fgRgb[0]},${fgRgb[1]},${fgRgb[2]});`
            };
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
        let currentReversed = false;

        // Resolves the style actually used to emit a span, applying the reverse-video
        // swap (see applyReverseVideo) when active. Both emission sites below call this
        // instead of reading currentClasses/currentFgStyle/currentBgStyle directly, so
        // the swap logic lives in exactly one place. The non-reversed path returns the
        // current state as-is (no extra work), so normal-case rendering is unaffected.
        function currentEffectiveStyle() {
            if (!currentReversed) return { classes: currentClasses, fgStyle: currentFgStyle, bgStyle: currentBgStyle };
            return applyReverseVideo(currentClasses, currentFgStyle, currentBgStyle);
        }

        let match;
        while ((match = ansiRegex.exec(text)) !== null) {
            // Add text before this escape sequence
            if (match.index > lastIndex) {
                const rawText = text.substring(lastIndex, match.index);

                // Resolve reverse-video (swaps fg/bg) before anything else uses the
                // style state - contrast adjustment and shade blending below then run
                // on top of the already-swapped colors, same as for any other color.
                const { classes: effClasses, fgStyle: effFgStyle, bgStyle: effBgStyle } = currentEffectiveStyle();

                // Apply color contrast adjustment if enabled
                let adjustedFgStyle = effFgStyle;
                if (colorOffsetPercent > 0) {
                    const fgRgb = getFgRgb(effClasses, effFgStyle);
                    const bgRgb = getBgRgb(effClasses, effBgStyle);
                    const adjustedFg = adjustFgForContrast(fgRgb, bgRgb, colorOffsetPercent);
                    // Check if color was actually adjusted
                    if (adjustedFg[0] !== fgRgb[0] || adjustedFg[1] !== fgRgb[1] || adjustedFg[2] !== fgRgb[2]) {
                        adjustedFgStyle = `color:rgb(${adjustedFg[0]},${adjustedFg[1]},${adjustedFg[2]});`;
                    }
                }

                const classes = effClasses.length > 0 ? ` class="${effClasses.join(' ')}"` : '';
                const styles = (adjustedFgStyle || effBgStyle) ? ` style="${adjustedFgStyle}${effBgStyle}"` : '';

                // Check for shade characters that need blending
                const shadeResult = processShadeChars(rawText, effClasses, effFgStyle, effBgStyle);
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
                    currentReversed = false;
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
                } else if (code === 7) {
                    currentReversed = true;
                } else if (code === 27) {
                    currentReversed = false;
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

            // Resolve reverse-video first, same as the mid-text emission site above.
            const { classes: effClasses, fgStyle: effFgStyle, bgStyle: effBgStyle } = currentEffectiveStyle();

            // Apply color contrast adjustment if enabled
            let adjustedFgStyle = effFgStyle;
            if (colorOffsetPercent > 0) {
                const fgRgb = getFgRgb(effClasses, effFgStyle);
                const bgRgb = getBgRgb(effClasses, effBgStyle);
                const adjustedFg = adjustFgForContrast(fgRgb, bgRgb, colorOffsetPercent);
                // Check if color was actually adjusted
                if (adjustedFg[0] !== fgRgb[0] || adjustedFg[1] !== fgRgb[1] || adjustedFg[2] !== fgRgb[2]) {
                    adjustedFgStyle = `color:rgb(${adjustedFg[0]},${adjustedFg[1]},${adjustedFg[2]});`;
                }
            }

            const classes = effClasses.length > 0 ? ` class="${effClasses.join(' ')}"` : '';
            const styles = (adjustedFgStyle || effBgStyle) ? ` style="${adjustedFgStyle}${effBgStyle}"` : '';
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
        // name is restricted to real Discord emoji name characters (alphanumeric + underscore)
        // to prevent HTML/attribute injection via a crafted MUD line breaking out of the
        // alt="..."/title="..." attributes below.
        return html.replace(/&lt;(a?):([A-Za-z0-9_]+):(\d+)&gt;/g, function(match, animated, name, id) {
            const ext = animated ? 'gif' : 'png';
            const safeName = escapeHtml(name);
            const safeId = escapeHtml(id);
            const url = `https://cdn.discordapp.com/emojis/${safeId}.${ext}`;
            return `<img src="${url}" alt=":${safeName}:" title=":${safeName}:" class="discord-emoji" style="height: 1.2em; vertical-align: middle;">`;
        });
    }

    // Escape HTML
    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML.replace(/"/g, '&quot;').replace(/'/g, '&#39;');
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

            // Skip Discord custom emoji tags entirely (<:name:id> or <a:name:id>) - without
            // this, a run of text isn't allowed to reset at the tag's own '<'/'>' (neither is
            // a break char or whitespace), so two adjacent tags with no space between them
            // (or even one long-named tag on its own) get counted as one long "word". Once
            // that word crosses MIN_WORD_LEN, the next '_' encountered - which is very likely
            // to be inside the emoji name itself, since underscores are common there - gets a
            // ZWSP inserted right after it. That silently corrupts the tag's name and makes
            // convertDiscordEmojis's `[A-Za-z0-9_]+` regex fail to match it, leaving a broken
            // custom emoji rendered as literal (and now ZWSP-corrupted) text instead of the
            // image. Treating the whole tag as atomic - like the ANSI skip above - fixes this
            // regardless of tag length or adjacency to other tags.
            if (c === '<') {
                const tagMatch = /^<a?:[A-Za-z0-9_]+:\d+>/.exec(text.slice(i - 1));
                if (tagMatch) {
                    const rest = tagMatch[0].slice(1); // '<' already appended above
                    result += rest;
                    i += rest.length;
                    continue;
                }
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

    // --- Drag/wheel to reveal pending output ------------------------------------------
    //
    // In more-mode the newest output is held back and the only way through it used to be a
    // whole screenful at a time (PgDn/Tab). Scrolling could not reach it at all: when a line
    // is held back the client scrolls to the bottom, so the container has no scroll distance
    // left, a further downward drag moves nothing, and no `scroll` event is ever emitted.
    //
    // These handlers watch the gesture itself instead of the scroll position, so dragging past
    // the bottom feeds pending lines in one at a time, tracking the finger.
    //
    // Accumulated overscroll distance in CSS pixels, and how many rows of it have already
    // been spent. Both are running totals for the current gesture (see scheduleReveal).
    let revealAccumPx = 0;
    let revealConsumedRows = 0;
    let revealRafPending = false;
    let lastTouchY = null;

    function resetRevealAccum() {
        revealAccumPx = 0;
        revealConsumedRows = 0;
    }

    // Is a downward overscroll gesture meaningful right now? Only at the very bottom with
    // something actually held back - anywhere else the browser's own scrolling must be left
    // completely alone.
    function canRevealPending() {
        return isAtBottom() && pendingTotal() > 0;
    }

    // Turn accumulated drag distance into a release, at most one request per animation frame.
    //
    // Coalescing is what keeps this from being one WebSocket round-trip per line: a fast drag
    // covering five rows within a frame sends a single ReleasePending{count:5}, a slow drag
    // sends count:1 per row.
    //
    // Rows owed are recomputed from the running total each frame rather than subtracting the
    // consumed height as we go. Subtracting compounds floating-point error across frames and
    // silently swallows a row over a long drag (20 drags of 0.7 rows released 13 lines, not
    // 14); the epsilon then absorbs the representation error in the sum itself.
    function scheduleReveal() {
        if (revealRafPending) return;
        revealRafPending = true;
        requestAnimationFrame(() => {
            revealRafPending = false;
            const rowPx = lineHeightPx();
            if (rowPx <= 0) { resetRevealAccum(); return; }
            const wantRows = Math.floor(revealAccumPx / rowPx + 1e-9);
            let lines = wantRows - revealConsumedRows;
            if (lines <= 0) return;
            // Never ask for more than is actually held back, however hard the flick.
            const available = pendingTotal();
            if (lines >= available) {
                // Backlog exhausted: spend the whole gesture rather than banking credit that
                // would dump a burst the moment new output arrives.
                lines = available;
                resetRevealAccum();
            } else {
                revealConsumedRows = wantRows;
            }
            if (lines > 0) releaseLines(lines);
        });
    }

    // Wheel deltas are not always pixels. Firefox commonly reports DOM_DELTA_LINE, and treating
    // that raw value as pixels would make a single notch release a whole page.
    function wheelDeltaToPx(e) {
        if (e.deltaMode === 1) return e.deltaY * lineHeightPx();          // DOM_DELTA_LINE
        if (e.deltaMode === 2) return e.deltaY * elements.outputContainer.clientHeight; // DOM_DELTA_PAGE
        return e.deltaY;                                                  // DOM_DELTA_PIXEL
    }

    // Format count for status indicator (right-justified, 4 chars)
    function formatCount(n) {
        if (n >= 1000000) return 'Alot';
        if (n >= 10000) return (Math.min(Math.floor(n / 1000), 999) + 'K').padStart(4, ' ');
        return n.toString().padStart(4, ' ');
    }

    // Mirrors main.rs's World::has_activity(): a world counts as having activity
    // if it has unseen lines or held-back (paused/more-mode) pending lines. Was
    // previously shadowed by a second, unseen-only declaration later in this file
    // (hoisting made that one win everywhere, silently dropping the pending_count term
    // and disagreeing with the server's own ActivityUpdate whenever a background world
    // was more-mode paused) - that duplicate is now removed, this is the only definition.
    function worldHasActivity(w) {
        return ((w.unseen_lines || 0) > 0) || ((w.pending_count || 0) > 0);
    }

    // Update status bar
    function updateStatusBar() {
        const world = worlds[currentWorldIndex];

        // Remember the focused world so a cold start can restore it (see
        // persistLastActiveWorld() / the InitialState handler). This is the single
        // choke point called after every real focus change (switchWorldLocal,
        // WorldSwitchResult, WorldRemoved, InitialState itself), so no other call
        // site needs to persist separately. Cheap no-op unless the name changed.
        persistLastActiveWorld();

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
        const totalPending = pendingTotal();
        if (!isAtBottom() && !scrollRafPending) {
            const container = elements.outputContainer;
            const linesFromBottom = Math.floor((container.scrollHeight - container.scrollTop - container.clientHeight) / lineHeightPx());
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
                .filter((w, i) => i !== currentWorldIndex && worldHasActivity(w))
                .map(w => w.name);
            elements.activityIndicator.title = activeWorlds.length > 0
                ? 'Unseen: ' + activeWorlds.join(', ')
                : '';
        } else {
            elements.activityIndicator.style.display = 'none';
            elements.activityIndicator.title = '';
        }

        // Note icon: only shown when the current world actually has notes.
        elements.statusNoteBtn.style.display = (world && world.settings && world.settings.has_notes) ? '' : 'none';

        updateScrollbackProgress();
        renderTabsRibbon();
        renderIconBar();
        // Keep an open world-switch dropdown live - e.g. a disconnected
        // world's unseen count can reach zero (viewed from another client)
        // while the menu is still open, and it should drop out immediately
        // rather than waiting for the next open.
        if (worldMenuOpen) renderWorldMenu();
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

    // Connection log modal — persistent window showing each attempt with ✓/✗
    function shouldShowConnectionWindow() {
        return window.SHOW_CONNECTION_WINDOW === true ||
               window.SHOW_CONNECTION_WINDOW === 'true';
    }

    function showConnectionLog() {
        var modal = document.getElementById('connection-log-modal');
        if (modal) modal.style.display = 'flex';
    }

    function hideConnectionLog() {
        var modal = document.getElementById('connection-log-modal');
        if (modal) modal.style.display = 'none';
        // Clear rows on hide so a later legitimate re-show (a real subsequent failure)
        // never surfaces stale (lost)/(canceled)/success rows from this cycle.
        var list = document.getElementById('connection-log-list');
        if (list) list.innerHTML = '';
        var retryBtn = document.getElementById('connection-log-retry-btn');
        if (retryBtn) retryBtn.disabled = true;
    }

    function addConnectionAttempt(url, id) {
        var list = document.getElementById('connection-log-list');
        if (!list) return;
        var row = document.createElement('div');
        row.className = 'conn-attempt pending';
        row.dataset.attemptId = id;
        row.innerHTML = '<span class="conn-icon">⟳</span><span class="conn-url">' + url + '</span>';
        list.appendChild(row);
        list.scrollTop = list.scrollHeight;
    }

    function resolveAttempt(id, success, suffix) {
        var list = document.getElementById('connection-log-list');
        if (!list) return;
        var row = list.querySelector('[data-attempt-id="' + id + '"]');
        if (!row) return;
        row.classList.remove('pending');
        row.classList.add(success ? 'success' : 'failed');
        var icon = row.querySelector('.conn-icon');
        if (icon) icon.textContent = success ? '✓' : '✗';
        if (suffix) {
            var urlSpan = row.querySelector('.conn-url');
            if (urlSpan) urlSpan.textContent += ' ' + suffix;
        }
    }

    function enableConnectionLogRetry() {
        var btn = document.getElementById('connection-log-retry-btn');
        if (btn) btn.disabled = false;
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
            closeWorldMenu();
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
        // Open in Browser button is visible in all interfaces (not Android-only)
        // Share Logs is an Android bridge (the logs are in app-private storage there); on web
        // and the desktop GUI the same files are simply on the machine you are already using.
        const shareLogsRow = document.getElementById('cs-share-logs-row');
        if (shareLogsRow) shareLogsRow.style.display = isAndroid ? '' : 'none';
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
        // Hide New Window on Android (can't open browser tabs from the app)
        if (isAndroid) {
            document.querySelectorAll('[data-action="new-window"]').forEach(el => {
                el.style.display = 'none';
            });
        }
        // Hide Resync for master WebView GUI (it IS the master, resync is meaningless)
        if (window.WEBVIEW_MODE && window.AUTO_PASSWORD) {
            document.querySelectorAll('.menu-resync').forEach(el => {
                el.style.display = 'none';
            });
        }
    }

    // Update the auth key download button enabled state.
    // Requires both a key (from server or cached) AND a non-empty password field.
    function updateDownloadButtonState() {
        var dlBtn = document.getElementById('cs-auth-key-download');
        if (!dlBtn || dlBtn.style.display === 'none') return;
        var hasKey = !!(serverAuthKey || authKey);
        var passEl = document.getElementById('cs-password');
        var hasPassword = !!(passEl && passEl.value.trim());
        var canDownload = hasKey && hasPassword;
        dlBtn.disabled = !canDownload;
        dlBtn.style.opacity = canDownload ? '' : '0.4';
        dlBtn.title = !hasKey ? 'Connect to server first'
            : !hasPassword ? 'Enter password to enable download'
            : 'Save auth key to device';
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
            if (keyEl) keyEl.value = authKey || '';  // show saved key if one has been downloaded
            var modeEl = document.getElementById('cs-connection-mode');
            if (modeEl && typeof window.Android.getConnectionMode === 'function') {
                modeEl.value = window.Android.getConnectionMode() || 'auto';
            }
            var runModeEl = document.getElementById('cs-run-mode');
            var remoteFields = document.getElementById('cs-remote-fields');
            if (runModeEl && typeof window.Android.getRunMode === 'function') {
                var runMode = window.Android.getRunMode() || 'remote';
                runModeEl.value = runMode;
                if (remoteFields) remoteFields.style.display = (runMode === 'local') ? 'none' : '';
            }
            var sshEnabledEl = document.getElementById('cs-ssh-enabled');
            var sshFields = document.getElementById('cs-ssh-fields');
            if (sshEnabledEl && typeof window.Android.getSshEnabled === 'function') {
                var sshEnabled = !!window.Android.getSshEnabled();
                sshEnabledEl.value = sshEnabled ? 'yes' : 'no';
                if (sshFields) sshFields.style.display = sshEnabled ? '' : 'none';
            }
            var sshUserEl = document.getElementById('cs-ssh-user');
            if (sshUserEl) sshUserEl.value = (typeof window.Android.getSshUser === 'function') ? window.Android.getSshUser() : '';
            var sshPortEl = document.getElementById('cs-ssh-port');
            if (sshPortEl) sshPortEl.value = (typeof window.Android.getSshPort === 'function') ? (window.Android.getSshPort() || 22) : 22;
            var sshKeyEl = document.getElementById('cs-ssh-key');
            if (sshKeyEl) sshKeyEl.value = (typeof window.Android.getSshPrivateKey === 'function') ? window.Android.getSshPrivateKey() : '';
            var sshKeyPassEl = document.getElementById('cs-ssh-key-passphrase');
            if (sshKeyPassEl) sshKeyPassEl.value = (typeof window.Android.getSshKeyPassphrase === 'function') ? window.Android.getSshKeyPassphrase() : '';
            var sshPassEl = document.getElementById('cs-ssh-password');
            if (sshPassEl) sshPassEl.value = (typeof window.Android.getSshPassword === 'function') ? window.Android.getSshPassword() : '';
            var dlBtn = document.getElementById('cs-auth-key-download');
            if (dlBtn) dlBtn.textContent = 'Download';
            var errEl = document.getElementById('cs-auth-key-error');
            if (errEl) errEl.style.display = 'none';
            updateDownloadButtonState();
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
                    const pats = Array.isArray(action.patterns) ? action.patterns : (action.pattern ? [{ pattern: action.pattern }] : []);
                    const patternMatch = pats.some(p => (p.pattern || '').toLowerCase().includes(filterText));
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
            // Show first pattern; if multiple, note count
            const pats = Array.isArray(action.patterns) ? action.patterns : [];
            const firstPat = pats.length > 0 ? (pats[0].pattern || '') : (action.pattern || '');
            const patCount = pats.length || (action.pattern ? 1 : 0);
            patternSpan.textContent = patCount === 0 ? '(manual)'
                : patCount === 1 ? firstPat
                : firstPat + ' +' + (patCount - 1) + ' more';
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

    // Build the pattern rows in the inline action editor (patterns is a simple string array)
    function renderActionPatternRows(patterns) {
        const container = elements.actionPatternsContainer;
        container.innerHTML = '';
        if (patterns.length === 0) {
            const hint = document.createElement('div');
            hint.style.cssText = 'font-size:11px;color:#888;font-style:italic;padding:2px 0;';
            hint.textContent = 'No patterns — action runs only via /name';
            container.appendChild(hint);
        }
        patterns.forEach(function(pat, idx) {
            const row = document.createElement('div');
            row.style.cssText = 'display:flex;gap:4px;align-items:center;';

            const inp = document.createElement('input');
            inp.type = 'text';
            inp.className = 'form-input';
            inp.style.flex = '1';
            inp.value = pat;
            inp.placeholder = '^pattern$';
            inp.autocomplete = 'off';
            inp.addEventListener('input', function() { patterns[idx] = inp.value; });

            const del = document.createElement('button');
            del.className = 'btn btn-danger';
            del.style.cssText = 'padding:2px 6px;font-size:12px;flex-shrink:0;';
            del.textContent = '✕';
            del.addEventListener('click', function() {
                patterns.splice(idx, 1);
                renderActionPatternRows(patterns);
            });

            row.appendChild(inp);
            row.appendChild(del);
            container.appendChild(row);
        });
    }

    // Open Actions Editor popup
    function openActionsEditorPopup(editIndex) {
        actionsEditorPopupOpen = true;
        editingActionIndex = editIndex;
        elements.actionsListModal.className = 'modal';  // Hide list
        elements.actionsEditorModal.className = 'modal visible';

        // Track the live patterns array for this session
        let editPatterns;

        if (editIndex >= 0 && editIndex < actions.length) {
            // Editing existing action
            elements.actionEditorTitle.textContent = 'Edit Action';
            const action = actions[editIndex];
            elements.actionName.value = action.name || '';
            elements.actionWorld.value = action.world || '';
            // Set action-level match type
            elements.actionMatchType.value = action.match_type || 'Regexp';
            // Build live patterns array (simple strings) from action.patterns or legacy single pattern
            if (Array.isArray(action.patterns) && action.patterns.length > 0) {
                editPatterns = action.patterns.map(function(p) {
                    return typeof p === 'string' ? p : (p.pattern || '');
                });
            } else if (action.pattern) {
                editPatterns = [action.pattern];
            } else {
                editPatterns = [];
            }
            elements.actionCommand.value = action.command || '';
            elements.actionEnabled.value = (action.enabled !== false) ? 'yes' : 'no';
            elements.actionStartup.value = action.startup ? 'yes' : 'no';
            elements.actionGuiShortcut.value = action.gui_shortcut ? 'yes' : 'no';
            elements.actionSuppressBlanks.value = action.suppress_blanks ? 'yes' : 'no';
        } else {
            // New action
            elements.actionEditorTitle.textContent = 'New Action';
            elements.actionName.value = '';
            elements.actionWorld.value = '';
            elements.actionMatchType.value = 'Regexp';
            editPatterns = [''];
            elements.actionCommand.value = '';
            elements.actionEnabled.value = 'yes';
            elements.actionStartup.value = 'no';
            elements.actionGuiShortcut.value = 'no';
        }

        renderActionPatternRows(editPatterns);

        // Add pattern button
        elements.actionAddPatternBtn.onclick = function() {
            editPatterns.push('');
            renderActionPatternRows(editPatterns);
            // Focus the new input
            const rows = elements.actionPatternsContainer.querySelectorAll('input[type="text"]');
            if (rows.length > 0) rows[rows.length - 1].focus();
        };

        // Store editPatterns reference so saveAction() can read it
        elements.actionPatternsContainer._editPatterns = editPatterns;

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
        // Check for internal command conflicts - reuse the single source-of-truth
        // list (INTERNAL_COMMANDS) instead of a separately hand-maintained guess.
        if (isInternalCommand(name)) {
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

        // Collect patterns from live array (set up in openActionsEditorPopup)
        // Patterns are now simple strings; filter out empty ones
        const rawPatterns = elements.actionPatternsContainer._editPatterns || [];
        const filteredPatterns = rawPatterns.filter(function(p) {
            return (typeof p === 'string' ? p : (p.pattern || '')).trim() !== '';
        }).map(function(p) {
            return { pattern: typeof p === 'string' ? p : (p.pattern || '') };
        });

        const actionData = {
            name: name,
            world: elements.actionWorld.value.trim(),
            match_type: elements.actionMatchType.value || 'Regexp',
            patterns: filteredPatterns,
            command: elements.actionCommand.value,
            enabled: elements.actionEnabled.value === 'yes',
            startup: elements.actionStartup.value === 'yes',
            gui_shortcut: elements.actionGuiShortcut.value === 'yes',
            suppress_blanks: elements.actionSuppressBlanks.value === 'yes'
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
        var pagePath = basePath() + (page ? '/' + page : '/');
        if (window.SERVER_URL) {
            url = window.SERVER_URL + pagePath;
        } else {
            var proto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
            var host = window.WS_HOST || window.location.hostname;
            var port = (window.WS_PORT && window.WS_PORT !== 0) ? window.WS_PORT : window.location.port;
            url = proto + '://' + host + ':' + port + pagePath;
        }
        if (window.WEBVIEW_MODE) {
            sendIpc('open-url:' + url);
        } else if (typeof Android !== 'undefined' && Android.openExternalUrl) {
            Android.openExternalUrl(url);
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
        setupTabsMode = tabsMode;
        setupIconBarMode = iconBarMode;
        setupTlsProxy = tlsProxyEnabled;
        setupNewLineIndicator = newLineIndicator;
        setupKeyboardAlwaysVisible = keyboardAlwaysVisible;
        setupDebug = debugEnabled;
        setupArchive = scrollbackEnabled;
        setupLogInput = logInputEnabled;
        setupInputHeightValue = inputHeight;
        setupWrapspace = wrapspace;
        setupGuiTheme = guiTheme;
        setupColorOffset = colorOffsetPercent;
        setupTransparency = guiTransparency;
        // Load web edit state
        editPortMode = !httpEnabled ? 'disabled' : (httpPort === 9000 ? '9000' : 'custom');
        editCustomCert = tlsConfigured;
        if (elements.setupRemoteLinesInput) {
            elements.setupRemoteLinesInput.value = remoteInitialLines;
        }
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
        if (elements.setupTabsSelect) {
            elements.setupTabsSelect.value = setupTabsMode;
            updateCustomDropdown(elements.setupTabsSelect);
        }
        if (elements.setupIconBarSelect) {
            elements.setupIconBarSelect.value = setupIconBarMode;
            updateCustomDropdown(elements.setupIconBarSelect);
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
        if (setupKeyboardAlwaysVisible) {
            elements.setupKeyboardVisibleToggle.classList.add('active');
        } else {
            elements.setupKeyboardVisibleToggle.classList.remove('active');
        }
        if (setupDebug) {
            elements.setupDebugToggle.classList.add('active');
        } else {
            elements.setupDebugToggle.classList.remove('active');
        }
        if (setupArchive) {
            elements.setupArchiveToggle.classList.add('active');
        } else {
            elements.setupArchiveToggle.classList.remove('active');
        }
        // Log Input is only shown once Archive Input/Output is on - see
        // update_setup_visibility's doc comment in popup/definitions/setup.rs (the
        // console equivalent of this same conditional).
        if (elements.setupLogInputField) {
            elements.setupLogInputField.style.display = setupArchive ? 'flex' : 'none';
        }
        if (setupLogInput) {
            elements.setupLogInputToggle.classList.add('active');
        } else {
            elements.setupLogInputToggle.classList.remove('active');
        }
        // World switching dropdown
        elements.setupWorldSwitchSelect.value = setupWorldSwitchMode;
        updateCustomDropdown(elements.setupWorldSwitchSelect);
        // Input height stepper
        elements.setupInputHeightValue.textContent = setupInputHeightValue;
        // Wrap space stepper
        elements.setupWrapspaceValue.textContent = setupWrapspace;
        // Remote lines: plain text input, value set once on popup open (see openSettingsPopup)
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
            wrapspace: wrapspace,
            remote_initial_lines: remoteInitialLines,
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
            // Always true now — the server is always TLS-capable for remote clients
            // (auto cert or user-provided); kept only for GlobalSettingsMsg wire compat.
            web_secure: true,
            http_enabled: httpEnabled,
            http_port: httpPort,
            web_path: webPath,
            ws_enabled: wsEnabled,
            ws_port: wsPort,
            ws_cert_file: wsCertFile,
            ws_key_file: wsKeyFile,
            ws_password: wsPassword,
            tls_proxy_enabled: tlsProxyEnabled,
            zwj_enabled: zwjEnabled,
            tts_mode: ttsMode,
            tts_speak_mode: ttsSpeakMode,
            tabs: tabsMode,
            icon_bar: iconBarMode,
            new_line_indicator: newLineIndicator,
            mouse_enabled: mouseEnabled,
            debug_enabled: debugEnabled,
            dictionary_path: dictionaryPath,
            scrollback_enabled: scrollbackEnabled,
            log_input_enabled: logInputEnabled,
            keyboard_always_visible: keyboardAlwaysVisible
        };
    }

    // Send a full UpdateGlobalSettings snapshot to the server, but only once this
    // client has synced real values from the server at least once. Every global in
    // buildUpdateGlobalSettings() defaults to false/'' until a sync lands, so sending
    // before that would silently reset unrelated globals on the server (and, since
    // the server persists immediately, in ~/.clay/settings.dat). See CLAUDE.md /
    // settings-audit investigation for the incident this guards against.
    function sendGlobalSettings() {
        if (!settingsSynced) {
            console.warn('Clay: suppressed UpdateGlobalSettings push before initial settings sync');
            return;
        }
        send(buildUpdateGlobalSettings());
    }

    function saveSettingsAll() {
        // Clay Server tab (Android only) — save to SharedPreferences and reload
        if (settingsActiveTab === 'clay-server' && window.Android) {
            var runMode = ((document.getElementById('cs-run-mode') || {}).value || 'remote');
            if (typeof window.Android.setRunMode === 'function') window.Android.setRunMode(runMode);
            if (runMode === 'local') {
                // No remote fields to validate/save — the local server needs no configuration.
                if (typeof window.Android.reloadPage === 'function') window.Android.reloadPage();
                return;
            }

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
                window.SKIP_CONNECT = false;
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
            var connectionMode = ((document.getElementById('cs-connection-mode') || {}).value || 'auto').trim();
            if (typeof window.Android.saveConnectionMode === 'function') window.Android.saveConnectionMode(connectionMode);

            var sshEnabled = ((document.getElementById('cs-ssh-enabled') || {}).value === 'yes');
            if (typeof window.Android.saveSshEnabled === 'function') window.Android.saveSshEnabled(sshEnabled);
            var sshUser = ((document.getElementById('cs-ssh-user') || {}).value || '').trim();
            if (typeof window.Android.saveSshUser === 'function') window.Android.saveSshUser(sshUser);
            var sshPort = parseInt((document.getElementById('cs-ssh-port') || {}).value, 10);
            if (!Number.isFinite(sshPort) || sshPort <= 0) sshPort = 22;
            if (typeof window.Android.saveSshPort === 'function') window.Android.saveSshPort(sshPort);
            var sshKey = (document.getElementById('cs-ssh-key') || {}).value || '';
            if (sshKey.trim()) {
                if (typeof window.Android.saveSshPrivateKey === 'function') window.Android.saveSshPrivateKey(sshKey);
            } else if (typeof window.Android.clearSshPrivateKey === 'function') {
                window.Android.clearSshPrivateKey();
            }
            var sshKeyPassphrase = (document.getElementById('cs-ssh-key-passphrase') || {}).value || '';
            if (sshKeyPassphrase) {
                if (typeof window.Android.saveSshKeyPassphrase === 'function') window.Android.saveSshKeyPassphrase(sshKeyPassphrase);
            } else if (typeof window.Android.clearSshKeyPassphrase === 'function') {
                window.Android.clearSshKeyPassphrase();
            }
            var sshPassword = (document.getElementById('cs-ssh-password') || {}).value || '';
            if (sshPassword) {
                if (typeof window.Android.saveSshPassword === 'function') window.Android.saveSshPassword(sshPassword);
            } else if (typeof window.Android.clearSshPassword === 'function') {
                window.Android.clearSshPassword();
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
        if (setupWrapspace < 0) setupWrapspace = 0;
        if (setupWrapspace > 20) setupWrapspace = 20;

        moreModeEnabled = setupMoreMode;
        worldSwitchMode = setupWorldSwitchMode;
        ansiMusicEnabled = setupAnsiMusic;
        zwjEnabled = setupZwj;
        ttsMode = setupTtsMode.toLowerCase();
        applyTabsMode(setupTabsMode);
        applyIconBarMode(setupIconBarMode);
        tlsProxyEnabled = setupTlsProxy;
        newLineIndicator = setupNewLineIndicator;
        keyboardAlwaysVisible = setupKeyboardAlwaysVisible;
        applyKeyboardForceState();
        debugEnabled = setupDebug;
        scrollbackEnabled = setupArchive;
        logInputEnabled = setupLogInput;
        guiTheme = setupGuiTheme;
        colorOffsetPercent = setupColorOffset;
        wrapspace = setupWrapspace;
        applyTheme(guiTheme);
        setInputHeight(setupInputHeightValue);
        applyTransparency(setupTransparency);
        applyWrapspace(wrapspace);
        renderOutput();

        // Save web settings (skip if multiuser)
        if (!multiuserMode) {
            httpEnabled = editPortMode !== 'disabled';
            httpPort = editPortMode === 'custom' ? (parseInt(elements.webCustomPort.value) || 9000) : 9000;
            webPath = elements.webPath ? elements.webPath.value.replace(/^\/+|\/+$/g, '').replace(/[^A-Za-z0-9_-]/g, '') : webPath;
            wsAllowList = elements.webAllowList.value;
            wsPassword = elements.webWsPassword ? elements.webWsPassword.value : wsPassword;
            tlsConfigured = editCustomCert;
            wsCertFile = editCustomCert ? elements.webCertFile.value : '';
            wsKeyFile = editCustomCert ? elements.webKeyFile.value : '';

            var setupRemoteInitialLines = parseInt(elements.setupRemoteLinesInput ? elements.setupRemoteLinesInput.value : '', 10);
            if (!Number.isFinite(setupRemoteInitialLines)) setupRemoteInitialLines = 100;
            remoteInitialLines = Math.max(10, Math.min(5000, setupRemoteInitialLines));
        }

        // Save font settings
        _saveFontSettingsInline();

        // Send combined update to server
        if (settingsSynced) {
            const msg = buildUpdateGlobalSettings();
            msg.input_height = setupInputHeightValue;
            send(msg);
        } else {
            console.warn('Clay: suppressed UpdateGlobalSettings push before initial settings sync');
        }

        closeSettingsPopup();
    }

    function updateWebPopupUI() {
        // Update Port select (use edit state)
        elements.webPortSelect.value = editPortMode;
        elements.webCustomPortField.style.display = editPortMode === 'custom' ? 'flex' : 'none';
        elements.webCustomPort.value = httpPort;

        if (elements.webPath) elements.webPath.value = webPath;
        elements.webAllowList.value = wsAllowList;
        if (elements.webWsPassword) elements.webWsPassword.value = wsPassword;

        // Custom Cert File select (use edit state)
        elements.webCustomCertSelect.value = editCustomCert ? 'yes' : 'no';
        // Show placeholder if a custom cert is configured but paths not sent from server
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

        // Show/hide cert/key fields based on Custom Cert File selection
        elements.tlsCertField.style.display = editCustomCert ? 'flex' : 'none';
        elements.tlsKeyField.style.display = editCustomCert ? 'flex' : 'none';

        // Populate auth key field (read-only — use Modify Key to change it)
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

    // Add raw output lines (without %% prefix)
    function addRawOutputLines(lines, worldIndex) {
        const ts = Math.floor(Date.now() / 1000);
        if (worldIndex >= 0 && worldIndex < worlds.length) {
            lines.forEach(line => {
                const lineIndex = worlds[worldIndex].output_lines.length;
                worlds[worldIndex].output_lines.push({ text: line, ts: ts, from_server: false });
                if (worldIndex === currentWorldIndex) {
                    appendNewLine(line, ts, worldIndex, lineIndex, false, false);
                }
            });
        }
    }

    // Calculate next keepalive time (based only on last send time)
    function formatNextKA(lastSendSecs, lastRecvSecs) {
        const KEEPALIVE_SECS = 5 * 60; // 5 minutes
        const elapsed = lastSendSecs !== null && lastSendSecs !== undefined ? lastSendSecs : KEEPALIVE_SECS;
        const remaining = Math.max(0, KEEPALIVE_SECS - elapsed);
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
    // Guarded entry point. Keyboard actions and most buttons funnel through here, so a throw
    // in any single action would otherwise be an unhandled exception with no usable report on
    // Android (see guard()). The action id is included so the banner names what failed.
    function dispatchAction(actionId) {
        try {
            return dispatchActionImpl(actionId);
        } catch (e) {
            __clayShowError("action '" + actionId + "' threw: " + __clayErrText(e));
            return false;
        }
    }

    function dispatchActionImpl(actionId) {
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
                    if (pendingTotal() === 0) {
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
                if (pendingTotal() > 0) {
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
                if (pendingTotal() > 0) {
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
                    const redrawWorld = worlds[currentWorldIndex];
                    redrawWorld.output_lines = redrawWorld.output_lines.filter(l => l.from_server !== false);
                    for (const l of redrawWorld.output_lines) {
                        if (l.display_id === myDisplayId) l.display_id = null;
                    }
                    // Drop OUR OWN ▶ markers on what's now on screen - the local equivalent
                    // of the console's Ctrl+L. Local-only optimistic update (there's no
                    // ClayCommand for "redraw"), and correctly so: releasing our markers must
                    // not touch another instance's, which under per-line ownership it
                    // structurally cannot - we only ever clear entries matching our own id.
                    worldOutputCache[currentWorldIndex] = {};
                }
                renderOutput();
                return true;
            case 'reload':
                // Local only — never restart the remote server
                if (window.WEBVIEW_MODE) {
                    sendIpc('reload');
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
                // show_tags is server-owned state (broadcast to all clients on
                // change) - send /tag rather than flipping it locally, so the
                // ShowTagsChanged broadcast is the single source of truth.
                send({ type: 'SendCommand', world_index: currentWorldIndex, command: '/tag' });
                return true;
            case 'filter_popup':
                if (filterPopupOpen) closeFilterPopup(); else openFilterPopup();
                return true;
            case 'search_popup':
                if (searchPopupOpen) closeSearchPopup(); else openSearchPopup();
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

    // Connected worlds, each tagged with its index into the `worlds` array —
    // the single source of truth for the tabs ribbon (only ever shows worlds
    // that are currently connected). The world-switch dropdown uses its own,
    // wider helper below (getWorldSwitcherWorlds()) — don't repurpose this one
    // for it, or the ribbon's deliberately-connected-only behavior breaks too.
    function getConnectedWorlds() {
        return worlds
            .map((w, i) => ({ world: w, index: i }))
            .filter(w => w.world.connected);
    }

    // Worlds shown in the world-switch dropdown: connected worlds, plus any
    // disconnected world that still has unseen output pending. A disconnected
    // world drops out again once its unseen count reaches zero (e.g. viewed
    // from another client) - see the worldMenuOpen refresh hook in
    // updateStatusBar() for how that happens live while the menu is open.
    function getWorldSwitcherWorlds() {
        return worlds
            .map((w, i) => ({ world: w, index: i }))
            .filter(w => w.world.connected || (w.world.unseen_lines || 0) > 0);
    }

    // Rebuild the tabs-ribbon strip from the current connected-worlds list and
    // highlight whichever tab matches currentWorldIndex. Called from
    // updateStatusBar() so it always stays in sync with the rest of the UI
    // without needing its own scattered call sites.
    function renderTabsRibbon() {
        if (!elements.tabsRibbon) return;
        const connected = getConnectedWorlds();
        if (tabsMode === 'none' || connected.length === 0) {
            elements.tabsRibbon.style.display = 'none';
            return;
        }
        elements.tabsRibbon.style.display = 'flex';

        elements.tabsRibbonTabs.innerHTML = '';
        connected.forEach(({ world, index }) => {
            // No connection dot — this list is already filtered to connected
            // worlds only (getConnectedWorlds()), so it would always render
            // the same "on" state; pure noise on what should read as a tab.
            const tab = document.createElement('div');
            tab.className = 'tabs-ribbon-tab' + (index === currentWorldIndex ? ' active' : '');
            tab.textContent = world.name;
            tab.onclick = function(e) {
                e.stopPropagation();
                switchWorldLocal(index);
            };
            elements.tabsRibbonTabs.appendChild(tab);
        });

        // Only show the scroll arrows when the strip actually overflows.
        const overflowing = elements.tabsRibbonTabs.scrollWidth > elements.tabsRibbonTabs.clientWidth;
        elements.tabsRibbonLeft.classList.toggle('hidden', !overflowing);
        elements.tabsRibbonRight.classList.toggle('hidden', !overflowing);
    }

    // Apply a Tabs setting change: move the ribbon to the right place in the
    // DOM (top: before the output area; bottom: its natural position directly
    // above the status bar) and show/hide/re-render it. DOM position is moved
    // rather than juggled via flex `order` on every sibling — simpler and it
    // can't accidentally affect unrelated layout.
    function applyTabsMode(mode) {
        tabsMode = mode;
        if (!elements.tabsRibbon) return;
        const app = document.getElementById('app');
        if (mode === 'top') {
            app.insertBefore(elements.tabsRibbon, elements.outputContainer);
        } else {
            // 'bottom' (or 'none', where position doesn't matter since hidden)
            app.insertBefore(elements.tabsRibbon, elements.statusBar);
        }
        renderTabsRibbon();
    }

    // Whether the icon bar (Worlds/Actions/Settings/Find tiles + shortcuts,
    // see renderIconBar()) should be visible right now: the user setting,
    // combined with the current effective device type. Mirrors
    // keyboardForceEnabled()'s "setting && device check" shape.
    function iconBarVisible() {
        if (iconBarMode === 'none') return false;
        if (iconBarMode === 'all') return true;
        // 'app_tablet': the desktop WebView GUI app, or a tablet-width
        // layout - NOT a plain desktop browser tab, NOT phone.
        return deviceMode === 'tablet' || (deviceMode === 'desktop' && window.WEBVIEW_MODE);
    }

    // Apply an Icon Bar setting change and re-render.
    function applyIconBarMode(mode) {
        iconBarMode = mode;
        renderIconBar();
    }

    // Actions the user has flagged to show as a one-click shortcut tile in
    // the icon bar. Disabled actions are excluded - a disabled action can't
    // fire anyway, so showing it as a clickable shortcut would be misleading.
    function getShortcutActions() {
        return actions.filter(a => a.enabled && a.gui_shortcut);
    }

    // Rebuild the icon bar: show/hide it per iconBarVisible(), and - only
    // if there's at least one shortcut-enabled action - build the shortcut
    // tiles and show the ‹/› cycle arrows when they overflow. The 4
    // built-in tiles (Worlds/Actions/Settings/Find) are static markup in
    // index.html and never move; everything right of them (divider, cycle
    // arrows, shortcuts) is hidden entirely with zero shortcuts configured.
    function renderIconBar() {
        if (!elements.iconBar) return;
        const visible = iconBarVisible();
        elements.iconBar.style.display = visible ? 'flex' : 'none';
        if (!visible) return;

        const shortcuts = getShortcutActions();
        const hasShortcuts = shortcuts.length > 0;

        elements.iconBarDivider.style.display = hasShortcuts ? '' : 'none';
        elements.iconBarShortcuts.style.display = hasShortcuts ? '' : 'none';
        if (!hasShortcuts) {
            elements.iconBarLeft.style.display = 'none';
            elements.iconBarRight.style.display = 'none';
            elements.iconBarShortcuts.innerHTML = '';
            return;
        }

        elements.iconBarLeft.style.display = '';
        elements.iconBarRight.style.display = '';
        elements.iconBarShortcuts.innerHTML = '';
        shortcuts.forEach((action) => {
            const tile = document.createElement('div');
            tile.className = 'icon-tile shortcut';
            tile.title = action.name;
            const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
            svg.setAttribute('viewBox', '0 0 32 32');
            svg.setAttribute('fill', 'currentColor');
            const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
            path.setAttribute('d', 'M16 3 19.06 11.8 28.35 11.98 20.94 17.6 23.65 26.53 16 21.2 8.35 26.53 11.06 17.6 3.65 11.98 12.94 11.8Z');
            svg.appendChild(path);
            tile.appendChild(svg);
            const label = document.createElement('span');
            label.className = 'icon-tile-label';
            label.textContent = action.name;
            tile.appendChild(label);
            tile.onclick = function(e) {
                e.stopPropagation();
                invokeShortcutAction(action);
            };
            elements.iconBarShortcuts.appendChild(tile);
        });

        // Only show the cycle arrows when the shortcuts strip actually overflows -
        // same overflow check renderTabsRibbon() uses for its own arrows.
        const overflowing = elements.iconBarShortcuts.scrollWidth > elements.iconBarShortcuts.clientWidth;
        elements.iconBarLeft.classList.toggle('hidden', !overflowing);
        elements.iconBarRight.classList.toggle('hidden', !overflowing);
    }

    // Invoke a shortcut Action the same way manual invocation already works
    // elsewhere in the client (an action with no pattern is invoked as
    // /name, see actions.rs) - route through the exact same input pipeline
    // as typing it and pressing Enter, so history/more-mode/etc. all behave
    // identically to a real typed command.
    function invokeShortcutAction(action) {
        elements.input.value = '/' + action.name;
        sendCommand();
    }

    // Reflect the current showTags state on the icon bar's Toggle Tags
    // tile, so it reads as a real toggle (on/off), not a one-shot action
    // like the other built-in tiles. Called from every showTags-assignment
    // site (InitialState, GlobalSettingsUpdated, ShowTagsChanged).
    function updateTagsTileState() {
        if (elements.iconBarTagsTile) {
            elements.iconBarTagsTile.classList.toggle('active', showTags);
        }
    }

    // World-switch dropdown: opened by clicking the world name on the status
    // bar. Populated fresh from getWorldSwitcherWorlds() on every open (the
    // list can change while the app is running), unlike the static hamburger
    // dropdown. Reuses .menu-dropdown/.menu-item styling and the same
    // anchor-above-the-button positioning as toggleMenu().
    function toggleWorldMenu() {
        worldMenuOpen = !worldMenuOpen;
        if (worldMenuOpen) {
            // Opening this menu - dismiss the hamburger menu if it's up.
            // Both trigger buttons stopPropagation() on click, so neither
            // ever reaches the document.body.onclick handler that would
            // otherwise close the other one - has to be done explicitly here.
            if (menuOpen) closeMenu();
            renderWorldMenu();
            const rect = elements.statusItem.getBoundingClientRect();
            elements.worldMenuDropdown.style.bottom = (window.innerHeight - rect.top + 4) + 'px';
            elements.worldMenuDropdown.style.left = rect.left + 'px';
        }
        elements.worldMenuDropdown.classList.toggle('visible', worldMenuOpen);
    }

    function closeWorldMenu() {
        worldMenuOpen = false;
        elements.worldMenuDropdown.classList.remove('visible');
    }

    function renderWorldMenu() {
        const list = getWorldSwitcherWorlds();
        // Only hide the current world once the list is large enough that
        // trimming it actually helps - with 5 or fewer entries, show
        // everything including the world you're already looking at.
        const hideCurrent = list.length > 5;
        elements.worldMenuDropdown.innerHTML = '';
        list.forEach(({ world, index }) => {
            if (hideCurrent && index === currentWorldIndex) return;

            const item = document.createElement('div');
            item.className = 'menu-item' + (index === currentWorldIndex ? ' selected' : '');
            item.dataset.index = index;

            // Status bubble: same green/red connected convention as the
            // status bar's own dot (updateStatusBar()) - meaningful here
            // since this list can include disconnected-with-unseen worlds.
            const dot = document.createElement('span');
            dot.className = 'status-dot' + (world.connected ? '' : ' off');
            item.appendChild(dot);

            item.appendChild(document.createTextNode(world.name));

            const unseen = world.unseen_lines || 0;
            if (unseen > 0) {
                const badge = document.createElement('span');
                badge.className = 'shortcut';
                badge.textContent = '[' + unseen + ' unseen]';
                item.appendChild(badge);
            }

            elements.worldMenuDropdown.appendChild(item);
        });
    }

    elements.worldMenuDropdown && (elements.worldMenuDropdown.onclick = function(e) {
        e.stopPropagation();
        const item = e.target.closest('.menu-item');
        if (item) {
            switchWorldLocal(parseInt(item.dataset.index, 10));
            closeWorldMenu();
        }
    });

    // Toggle menu dropdown (unified - opens upward from button)
    function toggleMenu(anchorBtn) {
        menuOpen = !menuOpen;
        if (menuOpen) {
            // Opening this menu - dismiss the world-switch dropdown if it's
            // up, for the same stopPropagation reason as toggleWorldMenu().
            if (worldMenuOpen) closeWorldMenu();
            if (anchorBtn) {
                // Position dropdown above the anchor button
                const rect = anchorBtn.getBoundingClientRect();
                elements.menuDropdown.style.bottom = (window.innerHeight - rect.top + 4) + 'px';
                elements.menuDropdown.style.left = rect.left + 'px';
            }
        }
        elements.menuDropdown.classList.toggle('visible', menuOpen);
    }

    // Close menu dropdown
    function closeMenu() {
        menuOpen = false;
        elements.menuDropdown.classList.remove('visible');
    }

    // Close every content/navigation popup that could be left open from a
    // previous menu-item click - e.g. clicking Worlds then Actions back to
    // back (icon bar or hamburger menu, both funnel through
    // handleMenuItem()) used to leave both open, since each open*Popup()
    // shows its own modal with no awareness of any other one. Each call is
    // guarded by that popup's own *Open flag so closing an already-closed
    // popup is a no-op rather than doing needless work (some close
    // functions also clear text/re-render). Deliberately excludes the auth
    // and password-change modals - those are security-relevant gates with
    // their own lifecycle, not menu-item navigation.
    //
    // `except` names the action about to be dispatched (e.g. 'filter' or
    // 'search') whose OWN popup should be left untouched here - Find/Search
    // toggle themselves closed when clicked while already open
    // (`if (xOpen) close(); else open();` in the switch below), and closing
    // it here first would make that check always see it as closed, silently
    // turning "click to close" into "close then instantly reopen".
    //
    // Note: isAnyModalOpen() (further down this file) is a similar but
    // block-scoped, read-only list used elsewhere for a different purpose
    // (it's also missing help/search) - not reachable from here, and not
    // worth unifying with this one for what is a narrowly-scoped fix.
    function closeAllPopups(except) {
        if (filterPopupOpen && except !== 'filter') closeFilterPopup();
        if (searchPopupOpen && except !== 'search') closeSearchPopup();
        if (helpPopupOpen) closeHelpPopup();
        if (actionsListPopupOpen) closeActionsListPopup();
        if (actionsEditorPopupOpen) closeActionsEditorPopup();
        if (actionsConfirmPopupOpen) closeActionsConfirmPopup();
        if (settingsPopupOpen) closeSettingsPopup();
        if (worldsPopupOpen) closeWorldsPopup();
        if (worldSelectorPopupOpen) closeWorldSelectorPopup();
        if (worldEditorPopupOpen) closeWorldEditorPopup();
        if (worldConfirmPopupOpen) closeWorldConfirmPopup();
    }

    // Handle menu item click
    function handleMenuItem(action) {
        closeMenu();
        closeAllPopups(action);
        switch (action) {
            case 'help':
                openHelpPopup();
                break;
            case 'worlds':
                ws.send(JSON.stringify({ type: 'RequestConnectionsList' }));
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
            case 'import':
                showImportDialog('');
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
                // show_tags is server-owned state - route through /tag like the
                // toggle_tags keybinding does, instead of mutating locally.
                send({ type: 'SendCommand', world_index: currentWorldIndex, command: '/tag' });
                focusInputWithKeyboard();
                break;
            case 'filter':
                if (filterPopupOpen) closeFilterPopup(); else openFilterPopup();
                break;
            case 'search':
                if (searchPopupOpen) closeSearchPopup(); else openSearchPopup();
                break;
            case 'reload':
                // Local only — never restart the remote server
                if (window.WEBVIEW_MODE) {
                    sendIpc('reload');
                } else {
                    window.location.reload();
                }
                break;
            case 'new-window':
                var nwProto = window.WS_PROTOCOL === 'wss' ? 'https' : 'http';
                var nwHost = window.WS_HOST || window.location.hostname;
                var nwPort = (window.WS_PORT && window.WS_PORT !== 0)
                    ? window.WS_PORT : window.location.port;
                var newWindowUrl = nwProto + '://' + nwHost + ':' + nwPort + basePath() + '/';
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
        lastGoodPassword = null;
        lastGoodUsername = null;
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
            sendGlobalSettings();
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

        // Font size "A" label - click to decrease by one (status bar)
        if (elements.fontSliderLabel) {
            elements.fontSliderLabel.addEventListener('click', function(e) {
                e.stopPropagation();
                setFontSize(currentFontSize - 1);
            });
        }

        // Font size "A" label - click to decrease by one (nav bar)
        if (elements.navFontSliderLabel) {
            elements.navFontSliderLabel.addEventListener('click', function(e) {
                e.stopPropagation();
                setFontSize(currentFontSize - 1);
            });
        }

        // Font size value label - click to increase by one (status bar)
        if (elements.fontSliderVal) {
            elements.fontSliderVal.addEventListener('click', function(e) {
                e.stopPropagation();
                setFontSize(currentFontSize + 1);
            });
        }

        // Font size value label - click to increase by one (nav bar)
        if (elements.navFontSliderVal) {
            elements.navFontSliderVal.addEventListener('click', function(e) {
                e.stopPropagation();
                setFontSize(currentFontSize + 1);
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

        // Up button - short press: NEXT world, long press (1s): prev history.
        //
        // The two directions are deliberately not symmetrical. Short press follows the
        // console's world-switch keys (Ctrl-Up/Shift-Up -> world_next, keybindings.rs), which
        // this client already honours on its own keyboard path; long press follows the input
        // area's plain Up = older command, which is both the console's behaviour and the
        // universal terminal convention. Short press used to call requestPrevWorld(), so the
        // button and Ctrl-Up disagreed about which way "up" goes.
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
                requestNextWorld();
            }
            elements.input.focus();
        }
        if (elements.navUpBtn) {
            elements.navUpBtn.addEventListener('mousedown', guard('navUp/start', upBtnStart));
            elements.navUpBtn.addEventListener('mouseup', guard('navUp/end', upBtnEnd));
            elements.navUpBtn.addEventListener('touchstart', guard('navUp/start', upBtnStart), { passive: false });
            elements.navUpBtn.addEventListener('touchend', guard('navUp/end', upBtnEnd), { passive: false });
        }

        // Down button - short press: PREVIOUS world, long press (1s): next history.
        // Mirror of the up button above; see there for why the two press durations follow
        // different conventions.
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
                requestPrevWorld();
            }
            elements.input.focus();
        }
        if (elements.navDownBtn) {
            elements.navDownBtn.addEventListener('mousedown', guard('navDown/start', downBtnStart));
            elements.navDownBtn.addEventListener('mouseup', guard('navDown/end', downBtnEnd));
            elements.navDownBtn.addEventListener('touchstart', guard('navDown/start', downBtnStart), { passive: false });
            elements.navDownBtn.addEventListener('touchend', guard('navDown/end', downBtnEnd), { passive: false });
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
            if (pendingTotal() > 0) {
                releaseScreenful();
            } else {
                const pageHeight = container.clientHeight * 0.9;
                container.scrollTop += pageHeight;
            }
            // Landing at the bottom with nothing held back means we're following live output
            // again. pendingTotal() covers both the local queue and the server's count - the
            // same predicate every other site uses.
            if (isAtBottom() && pendingTotal() === 0) {
                paused = false;
                linesSincePause = 0;
            }
            updateStatusBar();
        }

        if (elements.navPgUpBtn) {
            elements.navPgUpBtn.addEventListener('touchstart', function(e) {
                elements.input.focus();
            }, { passive: true });
            elements.navPgUpBtn.addEventListener('touchend', guard('navPgUp', function(e) {
                e.preventDefault();
                handlePgUp();
            }), { passive: false });
            elements.navPgUpBtn.addEventListener('click', guard('navPgUp', function(e) {
                handlePgUp();
            }));
        }

        if (elements.navPgDnBtn) {
            elements.navPgDnBtn.addEventListener('touchstart', function(e) {
                elements.input.focus();
            }, { passive: true });
            elements.navPgDnBtn.addEventListener('touchend', guard('navPgDn', function(e) {
                e.preventDefault();
                handlePgDn();
            }), { passive: false });
            elements.navPgDnBtn.addEventListener('click', guard('navPgDn', function(e) {
                handlePgDn();
            }));
        }

        // Click on More/History indicator to release pending lines
        elements.statusMore.addEventListener('click', guard('statusMore', function() {
            releaseScreenful();
        }));

        // Click on Activity indicator to switch to world with activity
        elements.activityIndicator.addEventListener('click', guard('activityIndicator', function() {
            requestNextWorld();
        }));

        // Click on the note icon to open the current world's notes (same
        // editor as typing /note).
        elements.statusNoteBtn.addEventListener('click', function() {
            openNoteEditor();
        });

        // Click on the world name to open the world-switch dropdown
        if (elements.statusItem) {
            elements.statusItem.addEventListener('click', function(e) {
                e.stopPropagation();
                toggleWorldMenu();
            });
        }

        // Tabs ribbon scroll arrows
        if (elements.tabsRibbonLeft) {
            elements.tabsRibbonLeft.addEventListener('click', function(e) {
                e.stopPropagation();
                elements.tabsRibbonTabs.scrollBy({ left: -120, behavior: 'smooth' });
            });
        }
        if (elements.tabsRibbonRight) {
            elements.tabsRibbonRight.addEventListener('click', function(e) {
                e.stopPropagation();
                elements.tabsRibbonTabs.scrollBy({ left: 120, behavior: 'smooth' });
            });
        }

        // Icon bar: the 4 built-in tiles reuse the hamburger menu's own
        // dispatch (same data-action strings handleMenuItem() already
        // switches on) instead of duplicating popup-opening logic.
        if (elements.iconBar) {
            elements.iconBar.querySelectorAll('.icon-tile[data-action]').forEach((tile) => {
                tile.addEventListener('click', function(e) {
                    e.stopPropagation();
                    handleMenuItem(tile.dataset.action);
                });
            });
        }
        if (elements.iconBarLeft) {
            elements.iconBarLeft.addEventListener('click', function(e) {
                e.stopPropagation();
                elements.iconBarShortcuts.scrollBy({ left: -120, behavior: 'smooth' });
            });
        }
        if (elements.iconBarRight) {
            elements.iconBarRight.addEventListener('click', function(e) {
                e.stopPropagation();
                elements.iconBarShortcuts.scrollBy({ left: 120, behavior: 'smooth' });
            });
        }

        // Track whether we're at the bottom (for resize handling)
        let wasAtBottomBeforeResize = true;

        // Update tracking on scroll
        elements.outputContainer.addEventListener('scroll', function() {
            wasAtBottomBeforeResize = isAtBottom();
        }, { passive: true });

        // Drag past the bottom to reveal pending output (see scheduleReveal above).
        //
        // Every handler is a no-op unless canRevealPending() holds, and preventDefault() is
        // only ever called in that same case - ordinary scrolling anywhere in the buffer must
        // stay fully native. touchmove/wheel are registered non-passive *because* of that
        // conditional preventDefault; the browser has to be told we might cancel.
        elements.outputContainer.addEventListener('touchstart', function(e) {
            lastTouchY = e.touches.length ? e.touches[0].clientY : null;
            resetRevealAccum();
        }, { passive: true });

        elements.outputContainer.addEventListener('touchmove', function(e) {
            if (!e.touches.length) return;
            const y = e.touches[0].clientY;
            if (lastTouchY === null) { lastTouchY = y; return; }
            // Finger moving UP drags the content up and pulls newer text in from below -
            // the same direction that scrolls toward the newest output.
            const dy = lastTouchY - y;
            lastTouchY = y;
            if (dy <= 0 || !canRevealPending()) return;
            revealAccumPx += dy;
            // Suppress the overscroll glow / rubber-band while we're consuming the gesture.
            e.preventDefault();
            scheduleReveal();
        }, { passive: false });

        function endRevealTouch() {
            lastTouchY = null;
            resetRevealAccum();
        }
        elements.outputContainer.addEventListener('touchend', endRevealTouch, { passive: true });
        elements.outputContainer.addEventListener('touchcancel', endRevealTouch, { passive: true });

        elements.outputContainer.addEventListener('wheel', function(e) {
            const dy = wheelDeltaToPx(e);
            if (dy <= 0 || !canRevealPending()) return;
            revealAccumPx += dy;
            e.preventDefault();
            scheduleReveal();
        }, { passive: false });

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
            if (worldMenuOpen && !e.target.closest('.status-item')) {
                closeWorldMenu();
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
                !importDialogOpen &&
                !importInsecureDialogOpen &&
                !e.target.closest('#status-bar') &&
                !e.target.closest('#nav-bar') &&
                !e.target.closest('.menu-dropdown') &&
                !e.target.closest('select')) {
                elements.input.focus();
            }
        };

        // Keep keyboard visible on mobile by refocusing input when it loses focus.
        // Listeners are always installed (device mode and the keyboard-visible
        // setting can both change at runtime); each mechanism below consults
        // keyboardForceEnabled() live rather than being gated once at setup time.
        {
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
                    importDialogOpen ||
                    importInsecureDialogOpen ||
                    menuOpen ||
                    worldMenuOpen;
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
                if (!keyboardForceEnabled()) return;
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
                    if (!keyboardForceEnabled()) return;
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
                    if (!keyboardForceEnabled()) return;
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
                if (!keyboardForceEnabled()) return;
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
                if (pendingTotal() === 0) {
                    paused = false;
                    linesSincePause = 0;
                    updateStatusBar();
                }
                // With pending held back we simply park here. This used to call releaseAll(),
                // so merely scrolling back down from history dumped the entire backlog at once
                // with no way to ask for less. Reaching the bottom is not a request for
                // anything; dragging further (see the touchmove/wheel handlers) feeds lines in
                // one at a time, and PgDn/Tab still take a screenful.
            }
            scheduleRenderWindowCheck();
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

        // Search input handler
        elements.searchInput.addEventListener('input', updateSearch);
        elements.searchInput.addEventListener('keydown', function(e) {
            if (e.key === 'Escape') {
                e.preventDefault();
                closeSearchPopup();
            } else if (e.key === 'F5') {
                e.preventDefault();
                closeSearchPopup();
            } else if (e.key === 'Enter') {
                e.preventDefault();
                advanceSearch();
            }
        });

        // Filter / search close button handlers
        if (elements.filterCloseBtn) {
            elements.filterCloseBtn.addEventListener('click', closeFilterPopup);
        }
        if (elements.searchCloseBtn) {
            elements.searchCloseBtn.addEventListener('click', closeSearchPopup);
        }

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
                    action === 'search_popup' ||
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

            // Handle world editor popup — a form (unlike the world *selector* list below), so
            // it needs no arrow-key list-navigation; just let every key reach its own <input>
            // fields normally, and handle Escape to close. Without this guard, keys typed while
            // editing (arrows, Backspace, Delete, Home/End, Tab — all bound to actions by
            // default in keybindings.rs) fall through to the document-level catch-all further
            // below and steal focus back to the main command line on nearly every keystroke.
            if (worldEditorPopupOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    closeWorldEditorPopup();
                }
                return;
            }

            // Handle /import dialog — same reasoning as worldEditorPopupOpen above: it's a
            // form with real <input> fields, so every key needs to reach them normally
            // (arrows, Backspace, Delete, Home/End, Tab are all bound to actions by default
            // in keybindings.rs and would otherwise fall through to the catch-all below and
            // steal focus back to the main command line on nearly every keystroke).
            if (importDialogOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    hideImportDialog();
                }
                return;
            }

            // Handle /import insecure-transport confirm — no text inputs, but still needs a
            // guard so its own Escape/click handling isn't shadowed by the catch-all below.
            if (importInsecureDialogOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    importInsecureDialogOpen = false;
                    pendingImportCredentials = null;
                    const dlg = document.getElementById('import-insecure-dialog');
                    if (dlg) dlg.style.display = 'none';
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
        elements.input.addEventListener('input', function(e) {
            resetCompletion();
            // Mirrors Rust's last_input_was_delete: any deleteContent*/deleteBy*
            // inputType counts as a deletion (allows undoing a conversion);
            // anything else (typed character, paste, IME composition) does not.
            lastInputWasDelete = !!(e.inputType && e.inputType.indexOf('delete') === 0);
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

        // Connection log modal buttons
        elements.connectionLogRetryBtn.onclick = function() {
            var list = document.getElementById('connection-log-list');
            if (list) list.innerHTML = '';
            elements.connectionLogRetryBtn.disabled = true;
            forceReconnect();
        };
        elements.connectionLogCancelBtn.onclick = function() {
            hideConnectionLog();
            if (typeof Android !== 'undefined' && Android.showFirstLaunchSetup) {
                Android.showFirstLaunchSetup();
            } else {
                openSettingsPopup('clay-server');
            }
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
        if (elements.actionEditorPageBtn) {
            elements.actionEditorPageBtn.onclick = function() { openEditorPage('action-editor'); };
        }

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
        var openBrowserSettingsBtn = document.getElementById('settings-open-browser-btn');
        if (openBrowserSettingsBtn) {
            openBrowserSettingsBtn.onclick = function() { openEditorPage(''); };
        }
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
        if (elements.setupTabsSelect) {
            elements.setupTabsSelect.onchange = function() {
                setupTabsMode = this.value;
            };
        }
        if (elements.setupIconBarSelect) {
            elements.setupIconBarSelect.onchange = function() {
                setupIconBarMode = this.value;
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
        elements.setupKeyboardVisibleToggle.onclick = function() {
            setupKeyboardAlwaysVisible = !setupKeyboardAlwaysVisible;
            updateSetupPopupUI();
        };
        elements.setupDebugToggle.onclick = function() {
            setupDebug = !setupDebug;
            updateSetupPopupUI();
        };
        elements.setupArchiveToggle.onclick = function() {
            setupArchive = !setupArchive;
            updateSetupPopupUI();
        };
        elements.setupLogInputToggle.onclick = function() {
            setupLogInput = !setupLogInput;
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
        elements.setupWrapspaceMinus.onclick = function() {
            if (setupWrapspace > 0) {
                setupWrapspace--;
                updateSetupPopupUI();
            }
        };
        elements.setupWrapspacePlus.onclick = function() {
            if (setupWrapspace < 20) {
                setupWrapspace++;
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

        // Note editor (NOTE_MODE only): save current world's notes and show a
        // brief confirmation. Save itself never closes the window — Cancel
        // (below) is the deliberate way to back out, whether or not you saved.
        if (elements.noteEditorSaveBtn) {
            elements.noteEditorSaveBtn.onclick = function() {
                if (!noteMode) return;
                send({
                    type: 'UpdateNote',
                    world_index: noteMode.world_index,
                    notes: elements.noteEditorTextarea.value
                });
                if (elements.noteEditorStatus) {
                    elements.noteEditorStatus.textContent = 'Saved';
                    elements.noteEditorStatus.classList.add('visible');
                    setTimeout(function() {
                        elements.noteEditorStatus.classList.remove('visible');
                    }, 1500);
                }
            };
        }

        // Cancel: discard any unsaved edits and leave note mode. Android
        // never actually opened a separate window (see enterNoteMode()), so
        // it just switches the current page back to the normal chat view;
        // desktop/plain-web really did spawn one, so they close it (same
        // IPC-vs-window.close() split as /quit).
        if (elements.noteEditorCancelBtn) {
            elements.noteEditorCancelBtn.onclick = function() {
                if (window.Android) {
                    exitNoteMode();
                } else if (window.WEBVIEW_MODE) {
                    sendIpc('close-window');
                } else {
                    window.close();
                }
            };
        }

        // Clay Server tab (Android only): toggle remote fields live when Run Mode changes,
        // without needing to save+reopen first.
        var csRunModeEl = document.getElementById('cs-run-mode');
        if (csRunModeEl) {
            csRunModeEl.onchange = function() {
                var remoteFields = document.getElementById('cs-remote-fields');
                if (remoteFields) remoteFields.style.display = (this.value === 'local') ? 'none' : '';
            };
        }

        // Clay Server tab (Android only): toggle SSH fields live when the SSH option changes.
        var csSshEnabledEl = document.getElementById('cs-ssh-enabled');
        if (csSshEnabledEl) {
            csSshEnabledEl.onchange = function() {
                var sshFields = document.getElementById('cs-ssh-fields');
                if (sshFields) sshFields.style.display = (this.value === 'yes') ? '' : 'none';
            };
        }

        // Web settings popup (use edit state, not global state)
        elements.webPortSelect.onchange = function() {
            editPortMode = this.value;
            updateWebPopupUI();
        };
        elements.webCustomCertSelect.onchange = function() {
            editCustomCert = this.value === 'yes';
            updateWebPopupUI();
        };
        // Modify Key button — opens the copy/regen/delete dialog
        if (elements.webModifyKeyBtn) {
            elements.webModifyKeyBtn.onclick = function() {
                showModifyKeyDialog();
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
                var keyToSave = serverAuthKey || authKey;
                if (!keyToSave || !window.Android) return;
                var passEl = document.getElementById('cs-password');
                var enteredPassword = (passEl ? passEl.value : '').trim();
                var errEl = document.getElementById('cs-auth-key-error');
                // Verify the entered password matches the server password
                if (!wsPassword) {
                    if (errEl) { errEl.textContent = 'Not connected — connect to server first'; errEl.style.display = ''; }
                    return;
                }
                if (enteredPassword !== wsPassword) {
                    if (errEl) { errEl.textContent = 'Incorrect password'; errEl.style.display = ''; }
                    return;
                }
                if (errEl) errEl.style.display = 'none';
                saveAuthKey(keyToSave);  // updates JS authKey var AND persists to Android storage
                // Show the saved key in the field so user can confirm it was stored
                var keyEl = document.getElementById('cs-auth-key');
                if (keyEl) keyEl.value = keyToSave;
                // Brief confirmation feedback on the button
                csAuthKeyDl.textContent = '✓ Saved';
                csAuthKeyDl.disabled = true;
                setTimeout(function() {
                    csAuthKeyDl.textContent = 'Download';
                    updateDownloadButtonState();
                }, 2000);
            };
        }
        // Migrated from the now-deleted SettingsActivity.java's "Clear Pinned
        // Certificates" button - same CertPinning.clearAllPins() call, now reachable
        // from the clay-server settings tab instead of the dead native Settings screen.
        var csShareLogsBtn = document.getElementById('cs-share-logs-btn');
        if (csShareLogsBtn) {
            csShareLogsBtn.onclick = function() {
                var status = document.getElementById('cs-share-logs-status');
                if (!window.Android || !window.Android.shareLogs) {
                    if (status) { status.textContent = 'Only available in the Android app.'; status.style.display = ''; }
                    return;
                }
                // shareLogs() reports what it found (or why it found nothing) synchronously and
                // opens the share sheet on the UI thread - surface that rather than leaving a
                // silent button when there are no logs to send.
                var msg = window.Android.shareLogs();
                if (status) { status.textContent = msg || ''; status.style.display = msg ? '' : 'none'; }
            };
        }

        var csClearPinsBtn = document.getElementById('cs-clear-pins-btn');
        if (csClearPinsBtn) {
            csClearPinsBtn.onclick = function() {
                if (!window.Android || !window.Android.clearAllPinnedCertificates) return;
                window.Android.clearAllPinnedCertificates();
                csClearPinsBtn.textContent = '✓ Cleared';
                setTimeout(function() { csClearPinsBtn.textContent = 'Clear Pinned Certificates'; }, 2000);
            };
        }
        var csPassEl = document.getElementById('cs-password');
        if (csPassEl) {
            csPassEl.oninput = function() {
                updateDownloadButtonState();
                var errEl = document.getElementById('cs-auth-key-error');
                if (errEl) errEl.style.display = 'none';
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

        // Keepalive ping every 30 seconds. Also piggybacks a PongCheck carrying our
        // current per-world ack (PROTOCOL-ROADMAP.md Step 5) so the server's acked_seq
        // (used to target ResyncRequired.from_seq on a channel-overflow resync, see
        // websocket.rs reconcile_resync) stays current between reconnects rather than
        // only being refreshed on an explicit /remote PingCheck. PongCheck is handled by
        // the server unconditionally (it doesn't require a matching PingCheck nonce to
        // land - see WsMessage::PongCheck handling in main.rs/daemon.rs), so sending it
        // proactively here is safe.
        setInterval(function() {
            if (ws && ws.readyState === WebSocket.OPEN && authenticated) {
                send({ type: 'Ping' });
                send({ type: 'PongCheck', nonce: 0, acked: buildResumeAckList() });
            }
        }, 30000);

        // Handle visibility change (browser sleep/wake)
        // When page becomes visible, ping the server to verify the connection is alive.
        // If pong arrives in time, resync. If not, reconnect.
        document.addEventListener('visibilitychange', function() {
            // Tell the server either way. Backgrounding is NOT a disconnect - the socket
            // usually stays open - so it has to be signalled explicitly: a hidden client
            // stops counting as a viewer and releases its ▶ markers, so text arriving while
            // it's away is unviewed and becomes ▶ when it comes back. Sent before the
            // early-return below so the hidden transition is reported at all.
            sendClientVisibility(document.visibilityState === 'visible');

            if (document.visibilityState !== 'visible') return;

            // If checkConnectionOnResume already started a wake check (or visibilitychange
            // itself fired earlier and one is still in flight), bail out — let the existing
            // wake check resolve via Pong or its 3s timeout. Prevents the resume race that
            // was tearing down healthy connections.
            if (wakeStateCleared) {
                debugLog('visibilitychange: wake check in progress, skipping');
                return;
            }

            if (!ws || ws.readyState === WebSocket.CLOSED) {
                forceReconnect();
            } else if (ws.readyState === WebSocket.CONNECTING) {
                // Already connecting — let it complete. The 5-second connection timeout
                // in connect() handles truly stale CONNECTING sockets.
                debugLog('visibilitychange: already connecting, skipping');
            } else if (ws.readyState === WebSocket.OPEN && !authenticated) {
                // Socket open but not authenticated — stale from before sleep
                forceReconnect();
            } else if (ws.readyState === WebSocket.OPEN && authenticated) {
                // Verify with a Ping. Use wakeStateCleared as a mutex so a follow-up
                // visibilitychange or checkConnectionOnResume call won't double-trigger.
                wakeStateCleared = true;
                try {
                    ws.send(JSON.stringify({ type: 'Ping' }));
                } catch (e) {
                    wakeStateCleared = false;
                    forceReconnect();
                    return;
                }
                wakePongTimeout = setTimeout(function() {
                    wakePongTimeout = null;
                    wakeStateCleared = false;
                    authenticated = false;
                    Object.keys(worlds).forEach(function(k) {
                        if (worlds[k]) worlds[k].connected = false;
                    });
                    updateStatusBar();
                    forceReconnect();
                }, 3000);
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

    // Show a blocking dialog for a TLS certificate pin mismatch on a MUD world
    // connection (trust-on-first-use, see platform::danger on the server). The
    // connection is already blocked server-side; this offers an explicit
    // "Trust new certificate" action that re-pins and reconnects.
    function showCertMismatchDialog(worldIndex, host, oldFingerprint, newFingerprint) {
        let dlg = document.getElementById('cert-mismatch-dialog');
        if (!dlg) {
            dlg = document.createElement('div');
            dlg.id = 'cert-mismatch-dialog';
            dlg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.6);z-index:3000;display:flex;align-items:center;justify-content:center;';
            document.body.appendChild(dlg);
        }
        const safeHost = escapeHtml(String(host));
        const safeOld = escapeHtml(String(oldFingerprint));
        const safeNew = escapeHtml(String(newFingerprint));
        dlg.innerHTML = sanitizeHtml(`
            <div style="background:#1a1a1a;color:#eee;border:2px solid #c00;border-radius:8px;padding:20px;max-width:500px;width:90%;">
                <div style="font-weight:bold;font-size:1.1em;margin-bottom:10px;color:#f55;">TLS Certificate Changed</div>
                <div style="margin-bottom:10px;">The certificate for <b>${safeHost}</b> no longer matches the one pinned on first connect. This could mean the server was reinstalled, or that someone is intercepting your connection.</div>
                <div style="font-family:monospace;font-size:0.85em;word-break:break-all;margin-bottom:6px;">Old: ${safeOld}</div>
                <div style="font-family:monospace;font-size:0.85em;word-break:break-all;margin-bottom:16px;">New: ${safeNew}</div>
                <div style="display:flex;gap:10px;justify-content:flex-end;">
                    <button id="cert-mismatch-cancel" style="padding:8px 16px;">Cancel</button>
                    <button id="cert-mismatch-trust" style="padding:8px 16px;background:#c00;color:#fff;border:none;border-radius:4px;">Trust new certificate</button>
                </div>
            </div>
        `);
        dlg.style.display = 'flex';

        document.getElementById('cert-mismatch-cancel').onclick = function() {
            hideCertMismatchDialog();
        };
        document.getElementById('cert-mismatch-trust').onclick = function() {
            send({ type: 'TrustCertificate', world_index: worldIndex, host: host, new_fingerprint: newFingerprint });
            hideCertMismatchDialog();
        };
    }

    function hideCertMismatchDialog() {
        const dlg = document.getElementById('cert-mismatch-dialog');
        if (dlg) {
            dlg.style.display = 'none';
        }
    }

    // Modify Auth Key dialog (opened from the web settings Auth Key row, which is
    // read-only — this is the only place the key can be changed). Same pattern as
    // showCertMismatchDialog above: a dynamically created full-screen overlay, styled
    // inline, content run through sanitizeHtml/escapeHtml. Regen/Delete take effect
    // immediately (server persists and broadcasts to every connected client), so this
    // dialog doesn't wait for the settings popup's own Save button.
    function copyTextToClipboard(text) {
        if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).catch(function() {});
            return;
        }
        // Fallback for contexts without the async clipboard API
        var ta = document.createElement('textarea');
        ta.value = text;
        ta.style.position = 'fixed';
        ta.style.opacity = '0';
        document.body.appendChild(ta);
        ta.select();
        try { document.execCommand('copy'); } catch (e) { /* best effort */ }
        document.body.removeChild(ta);
    }

    function showModifyKeyDialog() {
        let dlg = document.getElementById('modify-key-dialog');
        if (!dlg) {
            dlg = document.createElement('div');
            dlg.id = 'modify-key-dialog';
            dlg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.6);z-index:3000;display:flex;align-items:center;justify-content:center;';
            document.body.appendChild(dlg);
        }
        renderModifyKeyDialog(dlg);
        dlg.style.display = 'flex';
    }

    function renderModifyKeyDialog(dlg) {
        const keyText = serverAuthKey || '';
        const safeKey = escapeHtml(keyText || '(none)');
        dlg.innerHTML = sanitizeHtml(`
            <div style="background:#1a1a1a;color:#eee;border:2px solid #555;border-radius:8px;padding:20px;max-width:480px;width:90%;">
                <div style="font-weight:bold;font-size:1.1em;margin-bottom:10px;">Modify Auth Key</div>
                <div style="font-family:monospace;font-size:0.85em;word-break:break-all;margin-bottom:16px;opacity:0.9;">${safeKey}</div>
                <div style="display:flex;gap:10px;justify-content:flex-end;flex-wrap:wrap;">
                    <button id="modify-key-close" style="padding:8px 16px;">Close</button>
                    <button id="modify-key-delete" style="padding:8px 16px;background:#c00;color:#fff;border:none;border-radius:4px;">Delete</button>
                    <button id="modify-key-regen" style="padding:8px 16px;">Regen</button>
                    <button id="modify-key-copy" style="padding:8px 16px;">Copy</button>
                </div>
            </div>
        `);
        document.getElementById('modify-key-close').onclick = function() {
            hideModifyKeyDialog();
        };
        document.getElementById('modify-key-copy').onclick = function() {
            if (keyText) copyTextToClipboard(keyText);
        };
        document.getElementById('modify-key-regen').onclick = function() {
            send({ type: 'RegenerateAuthKey' });
            // Dialog content refreshes when the KeyGenerated response arrives.
        };
        document.getElementById('modify-key-delete').onclick = function() {
            send({ type: 'RevokeKey', auth_key: keyText });
            serverAuthKey = '';
            if (elements.webAuthKey) elements.webAuthKey.value = '';
            hideModifyKeyDialog();
        };
    }

    function hideModifyKeyDialog() {
        const dlg = document.getElementById('modify-key-dialog');
        if (dlg) {
            dlg.style.display = 'none';
        }
    }

    function isModifyKeyDialogOpen() {
        const dlg = document.getElementById('modify-key-dialog');
        return !!(dlg && dlg.style.display !== 'none');
    }

    // /import dialogs (plan i-d-like-to-make-snuggly-rain.md, step 7). Same pattern as
    // showCertMismatchDialog above: a dynamically created full-screen overlay, styled
    // inline, content run through sanitizeHtml/escapeHtml.

    // Stashed credentials from the last ImportSettings attempt, so the insecure-confirm
    // retry (allow_insecure: true) can resend them without prompting the user again.
    let pendingImportCredentials = null;

    function showImportDialog(prefillAddr) {
        importDialogOpen = true;
        let dlg = document.getElementById('import-dialog');
        if (!dlg) {
            dlg = document.createElement('div');
            dlg.id = 'import-dialog';
            dlg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.6);z-index:3000;display:flex;align-items:center;justify-content:center;';
            document.body.appendChild(dlg);
        }
        const safeAddr = escapeHtml(String(prefillAddr || ''));
        dlg.innerHTML = sanitizeHtml(`
            <div style="background:#1a1a1a;color:#eee;border:2px solid #555;border-radius:8px;padding:20px;max-width:420px;width:90%;">
                <div style="font-weight:bold;font-size:1.1em;margin-bottom:10px;">Import Settings</div>
                <div style="margin-bottom:12px;opacity:0.85;">Pull worlds, theme, and keybindings from another Clay instance. Remote values win on conflicts; everything else you have locally is kept.</div>
                <label style="display:block;margin-bottom:8px;">Host[:port]<br>
                    <input id="import-addr" type="text" value="${safeAddr}" style="width:100%;box-sizing:border-box;padding:6px;margin-top:4px;" autocomplete="off">
                </label>
                <label style="display:block;margin-bottom:8px;">Password<br>
                    <input id="import-password" type="password" style="width:100%;box-sizing:border-box;padding:6px;margin-top:4px;" autocomplete="off">
                </label>
                <label style="display:block;margin-bottom:16px;">Auth key (optional, instead of password)<br>
                    <input id="import-authkey" type="password" style="width:100%;box-sizing:border-box;padding:6px;margin-top:4px;" autocomplete="off">
                </label>
                <div style="display:flex;gap:10px;justify-content:flex-end;">
                    <button id="import-cancel" style="padding:8px 16px;">Cancel</button>
                    <button id="import-go" style="padding:8px 16px;background:#06c;color:#fff;border:none;border-radius:4px;">Import</button>
                </div>
            </div>
        `);
        dlg.style.display = 'flex';
        const addrInput = document.getElementById('import-addr');
        addrInput.focus();
        addrInput.select();

        function submit() {
            const addr = document.getElementById('import-addr').value.trim();
            const password = document.getElementById('import-password').value;
            const authKey = document.getElementById('import-authkey').value;
            if (!addr) return;
            if (!password && !authKey) {
                appendClientLine('Enter a password or an auth key.', currentWorldIndex, 'system');
                return;
            }
            pendingImportCredentials = { addr: addr, password: password || null, auth_key: authKey || null };
            send({ type: 'ImportSettings', addr: addr, password: password || null, auth_key: authKey || null, allow_insecure: false });
            hideImportDialog();
        }

        document.getElementById('import-cancel').onclick = hideImportDialog;
        document.getElementById('import-go').onclick = submit;
        document.getElementById('import-authkey').onkeydown = function(e) {
            if (e.key === 'Enter') submit();
        };
    }

    function hideImportDialog() {
        importDialogOpen = false;
        const dlg = document.getElementById('import-dialog');
        if (dlg) {
            dlg.style.display = 'none';
        }
    }

    function showImportInsecureConfirmDialog(addr) {
        importInsecureDialogOpen = true;
        let dlg = document.getElementById('import-insecure-dialog');
        if (!dlg) {
            dlg = document.createElement('div');
            dlg.id = 'import-insecure-dialog';
            dlg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.6);z-index:3000;display:flex;align-items:center;justify-content:center;';
            document.body.appendChild(dlg);
        }
        const safeAddr = escapeHtml(String(addr));
        dlg.innerHTML = sanitizeHtml(`
            <div style="background:#1a1a1a;color:#eee;border:2px solid #c00;border-radius:8px;padding:20px;max-width:460px;width:90%;">
                <div style="font-weight:bold;font-size:1.1em;margin-bottom:10px;color:#f55;">No Secure Connection</div>
                <div style="margin-bottom:16px;">${safeAddr} did not accept a TLS connection. Continuing will send your password/auth-key to it <b>unencrypted</b>. Only do this on a network you trust.</div>
                <div style="display:flex;gap:10px;justify-content:flex-end;">
                    <button id="import-insecure-cancel" style="padding:8px 16px;">Cancel</button>
                    <button id="import-insecure-go" style="padding:8px 16px;background:#c00;color:#fff;border:none;border-radius:4px;">Send unencrypted</button>
                </div>
            </div>
        `);
        dlg.style.display = 'flex';

        document.getElementById('import-insecure-cancel').onclick = function() {
            importInsecureDialogOpen = false;
            pendingImportCredentials = null;
            dlg.style.display = 'none';
        };
        document.getElementById('import-insecure-go').onclick = function() {
            importInsecureDialogOpen = false;
            dlg.style.display = 'none';
            if (pendingImportCredentials) {
                send(Object.assign({ type: 'ImportSettings', allow_insecure: true }, pendingImportCredentials));
                pendingImportCredentials = null;
            }
        };
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

    // Expose for Android Java calls via WebView.evaluateJavascript (which runs in window scope)
    window.connect = connect;
    window.openSettingsPopup = openSettingsPopup;

    // Called by Android when it silently restarts a dead SSH tunnel (network change/resume
    // watchdog - see MainActivity.restartSshTunnel()) and the new tunnel landed on a different
    // local port (SshProxyManager always picks a fresh ephemeral port on restart). Updates the
    // port we dial and reconnects - buildCandidates() already reads window.WS_PORT fresh on
    // every call, so this is all that's needed; no other reconnect-path changes required.
    window.updateSshTunnelPort = function(newPort) {
        window.WS_PORT = newPort;
        forceReconnect();
    };

    // Called by native WebView GUI to show update status messages
    window.showUpdateStatus = function(msg) {
        appendClientLine(msg);
    };

    // Expose native WebSocket check for debugging
    window.isUsingNativeWebSocket = function() {
        return winnerAttemptId !== null;
    };

    // Called by Android when the 1-hour background shutdown timer fires
    window.onBackgroundTimeout = function() {
        debugLog('Background timeout - connection closed by Android');
        authenticated = false;
        connectionFailures = 0;
        winnerAttemptId = null;
        pendingAttempts.clear();
        // Don't auto-reconnect here - we're in the background and Android disconnected
        // to save power. Reconnection will happen when user returns (checkConnectionOnResume).
    };

    // Called by Android onResume when interface is loaded but not connected.
    // This handles cases where the connection died in the background (timeout,
    // silent TCP death, etc.) and the visibilitychange event may not fire.
    window.checkConnectionOnResume = function() {
        debugLog('checkConnectionOnResume: ws=' + (ws ? ws.readyState : 'null') + ' auth=' + authenticated + ' wakeStateCleared=' + wakeStateCleared);
        // Report what the socket looked like on return, and what we decided to do about it.
        // A reconnect here is expensive and user-visible, and the two causes need different
        // fixes: readyState 3 (CLOSED) means something outside the page killed the socket
        // while we were away, whereas OPEN-and-authenticated followed by a `pongTimeout`
        // event means the socket looked fine but the server never answered. On a phone with
        // no adb attached this is the only way to tell them apart.
        recordClientEvent('resumeCheck', 'ws=' + (ws ? ws.readyState : 'null')
            + ' auth=' + authenticated
            + ' wakeCheckInFlight=' + wakeStateCleared
            + ' connectInProgress=' + connectInProgress
            + ' awayMs=' + (lastHiddenAt ? (Date.now() - lastHiddenAt) : -1));
        // If visibilitychange (or an earlier checkConnectionOnResume call) already started
        // a wake check, defer — let it resolve. wakeStateCleared acts as the mutex.
        if (wakeStateCleared) {
            debugLog('checkConnectionOnResume: wake check already in progress, skipping');
            return;
        }
        // If parallel candidates are in flight, ws is still null — connectInProgress guards this.
        if (connectInProgress) {
            debugLog('checkConnectionOnResume: connect in progress, skipping');
            return;
        }
        if (ws && ws.readyState === WebSocket.CONNECTING) {
            debugLog('checkConnectionOnResume: reconnect already in progress, skipping');
            return;
        }
        if (!ws || ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) {
            // Connection is dead, reconnect
            recordClientEvent('resumeReconnect', 'reason=socketClosed ws='
                + (ws ? ws.readyState : 'null'));
            forceReconnect();
        } else if (ws.readyState === WebSocket.CONNECTING) {
            // Stale connecting attempt, kill and retry
            forceReconnect();
        } else if (ws.readyState === WebSocket.OPEN && !authenticated) {
            // Socket open but not authenticated - stale
            forceReconnect();
        } else if (ws.readyState === WebSocket.OPEN && authenticated) {
            // Looks connected — verify with Ping. Set wakeStateCleared so visibilitychange
            // (which may fire in the same resume event) skips its own forceReconnect.
            if (wakePongTimeout) {
                clearTimeout(wakePongTimeout);
                wakePongTimeout = null;
            }
            wakeStateCleared = true;
            try {
                ws.send(JSON.stringify({ type: 'Ping' }));
            } catch (e) {
                wakeStateCleared = false;
                forceReconnect();
                return;
            }
            wakePongTimeout = setTimeout(function() {
                wakePongTimeout = null;
                wakeStateCleared = false;
                // Pong never arrived — connection is stale; clear visual state then reconnect
                recordClientEvent('resumeReconnect', 'reason=pongTimeout');
                authenticated = false;
                Object.keys(worlds).forEach(function(k) {
                    if (worlds[k]) worlds[k].connected = false;
                });
                updateStatusBar();
                forceReconnect();
            }, 3000);
        }
    };

    // Start the app
    try { init(); } catch (e) { __clayShowError('init() threw: ' + __clayErrText(e)); }
})();
