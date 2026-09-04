# Keyboard Shortcuts

Clay's default keybindings follow TinyFugue (TF) 5.0's own defaults, with a
short, deliberate list of exceptions (`^Q`, `^R`, and a handful of Clay-only
extras — see the "Differences from TinyFugue" section of
`docs/markdown/06-tf-commands.md` and `TINYFUGUE-COMPAT.md`). Every binding
below — including every chord — is stored in `~/.clay/keybindings.dat` and can
be freely rebound there or from the browser-based keybind editor
(`/keybind-editor`, when the HTTP server is enabled — see `/web`). Rebind a
key to `UNBOUND` to remove a default without picking a replacement.

**Chords.** A chord is two or more keystrokes typed in quick succession that
together name one binding (`^X^R`, `Esc-^N`, `Esc-Left`). The moment a
keystroke could be the first half of a longer binding, Clay buffers it for up
to 500ms waiting for the next key; `^G` (the `bell` action, TF's own
chord/numeric-prefix cancel key) discards a buffered prefix immediately
without dispatching anything. If the window expires with nothing more typed,
the buffered prefix's own binding fires if it has one — a bare `Escape` with
nothing bound to it directly just rings the bell, the same as any other
unbound key.

**Key names.** `^X` = Ctrl+X; `Esc-x` = Escape then `x` (also written
`Alt-x`/`Meta-x`/`@x` — all four mean the same thing, and case is
significant: `Esc-j` and `Esc-J` are different bindings); `Ctrl-Up`/
`Shift-Tab`/`Alt-Down` = a real terminal modifier held with a named key (one
physical keystroke, distinct from `Esc-Up` which is Escape-then-Up as two
separate keystrokes). `keybindings.dat` and `/bind` also accept TinyFugue's
own raw spellings (`^[b`, `\033`, `\0x1B`, `\27`, raw terminal escape
sequences like `^[[1;5A`) — see `/help bind`.

## World Switching

| Key | Action |
|---|---|
| `Esc-Left` | Previous connected world (TF SOCKETB) |
| `Esc-Right` | Next connected world (TF SOCKETF) |
| `Esc-{` | Previous active world (Clay's unseen-first cycling) |
| `Esc-}` | Next active world (Clay's unseen-first cycling) |
| `Shift-Up` | Next world, cycling through all worlds (including disconnected) |
| `Shift-Down` | Previous world, cycling through all worlds |
| `Esc-w` | Switch to the world with activity (oldest pending → unseen output → previous world) |
| `^]` | Background all worlds (TF `/bg`) — a no-op in Clay's single-pane console, kept for scripting parity |

Two cycling styles, two key pairs. `Esc-Left`/`Esc-Right` are TF's own
SOCKETB/SOCKETF and step through *connected* worlds in list order.
`Esc-{`/`Esc-}` run Clay's `world_prev`/`world_next` — the
alphabetical-or-unseen-first behavior set by the "World Switching" setting in
`/setup`. TF binds all four to socket cycling; Clay gives the redundant pair
to its own cycling so neither style loses a default key. The
`world_previous`/`world_forward`/`recent_worlds` actions still have no default
key — bind them in `keybindings.dat` or the keybind editor.

## Input Editing

| Key | Action |
|---|---|
| `Left` / `^B` | Cursor left |
| `Right` / `^F` | Cursor right |
| `Esc-b` / `Ctrl-Left` | Word left |
| `Esc-f` / `Ctrl-Right` | Word right |
| `^A` / `Home` | Jump to start of line |
| `^E` / `End` | Jump to end of line |
| `Up` / `Down` | Cursor up/down (multi-line input) |
| `Backspace` | Delete backward |
| `Delete` / `^D` | Delete forward |
| `^W` | Delete word backward |
| `Esc-d` | Delete word forward |
| `Esc-Backspace` / `Esc-^H` / `^X^?` | Delete word backward, punctuation-delimited |
| `^K` | Kill to end of line |
| `^U` | Kill to start of line (TF's real `^U`; Clay's older "clear whole line" is `clear_line`, TF's own `/dokey DLINE`, unbound by default) |
| `^T` | Transpose the two characters before the cursor |
| `^V` | Literal next (insert the next keystroke as a literal character) |
| `^Y` | Yank (paste the most recent kill-ring entry) |
| `Esc-c` / `Esc-l` / `Esc-u` | Capitalize / lowercase / uppercase word |
| `Esc-Space` | Collapse surrounding spaces to one |
| `Esc-=` | Goto matching bracket (`()[]{}`) |
| `Esc-.` / `Esc-_` | Insert the last word of the previous history entry |
| `Esc-^E` | Expand line (substitute `%var`/`$[...]`/`$(...)` in the input line in place — TF `kb_expand_line`) |
| `Esc-Tab` | Command completion (`Tab` itself keeps its own more-mode-priority paging behavior — see Output/Scrollback below) |
| `Alt-Up` / `Alt-Down` | Grow/shrink the input area (1-15 lines) |
| `^Q` | Spell suggestions (Clay keeps this; TF's own `^Q` is literal-next, which Clay puts on `^V`) |

**Kill ring:** `^K`, `^U`, `^W`, `Esc-d`, and `Esc-Backspace` all push the
deleted text to the kill ring; `^Y` pastes the most recent entry.

## Numeric Prefix (kbnum)

TF's `%kbnum` (tf-help `#kbnum`): build up a repeat count or motion magnitude
one digit at a time before an action, the way a shell `readline` numeric
argument works.

| Key | Action |
|---|---|
| `Esc-0` … `Esc-9` | Add a digit to the pending numeric prefix |
| `Esc--` | Start (or negate) the pending numeric prefix |

The prefix shows as `[N]` next to the world name in the separator bar while
it's pending. It is consumed by cursor/word movement, delete, transpose,
capitalize/lowercase/uppercase word, history navigation, `recall_begin`/
`recall_end`, page/half-page/line scrolling, and world-socket cycling; a
negative prefix reverses the action's direction. `%kbnum` is cleared
automatically after the next action that isn't itself a digit or `Esc--`,
and by `^G`. The magnitude is clamped to TF's own documented `max_kbnum`
default of 999.

## Insert Mode

| Key | Action |
|---|---|
| `Insert` / `Esc-v` | Toggle insert/overwrite mode |

TF's own `Insert`/`Esc-v` toggle (`%insert`). While overwrite is active,
typing a character replaces the one under the cursor instead of pushing it
right (typing at the end of the line still appends, since there's nothing to
overwrite). Mirrored the same way in the web/GUI client.

## History

| Key | Action |
|---|---|
| `^P` / `Ctrl-Up` | Previous history entry |
| `^N` / `Ctrl-Down` | Next history entry |
| `Esc-p` | Search history backward (entries starting with the current input) |
| `Esc-n` | Search history forward (continues a backward search) |
| `Ctrl-Home` / `Esc-<` | Jump to the oldest history entry (TF RECALLBEG) |
| `Ctrl-End` / `Esc->` | Jump to the newest history entry (TF RECALLEND) |

`Ctrl-Up`/`Ctrl-Down` recall history under the TF-parity defaults — they used
to switch worlds; see "World Switching" above and `TINYFUGUE-COMPAT.md` for
the full list of changed defaults.

## Output / Scrollback

| Key | Action |
|---|---|
| `PageUp` | Scroll back (enables more-mode pause) |
| `PageDown` | Scroll forward (unpauses at the bottom) |
| `Tab` | Release one screenful of pending output when paused; command completion when the input starts with `/`; otherwise pages like `PageDown` |
| `Esc-j` | Jump to end, release all pending output |
| `Esc-J` | Selective flush: keep highlighted pending lines, discard the rest |
| `Esc-h` | Half-page scroll back, or release half a screenful of pending output |
| `Esc-^N` | Scroll forward one line (TF LINE) |
| `Esc-^P` | Scroll back one line (TF LINEBACK) |
| `Esc-^L` | Clear the view without dropping any lines — scrollback still holds them (TF CLEAR) |
| `^S` | Pause the current world so new output queues as pending (TF PAUSE) |
| `Ctrl-PageDown` | Same as `Esc-j` |
| `Esc-L` | Toggle the F4 filter popup between no limit and the last limit applied |
| `^X[` / `^X]` | Half-page back / half-page forward (TF's own `^X` prefix map) |
| `^X{` / `^X}` | Page back / page forward |
| `F4` | Open the filter popup to search output |

## General

| Key | Action |
|---|---|
| `F1` | Help |
| `F2` | Toggle MUD tag display (channel tags and timestamps) |
| `F5` | Search command history |
| `F8` | Toggle action-pattern highlighting |
| `F9` | Toggle GMCP media audio |
| `^G` | Bell — also cancels a buffered chord or a pending numeric prefix |
| `^L` | Redraw the screen, keeping only server output (drops client-generated lines). Unchanged from earlier Clay releases. TF's plain repaint is the separate `refresh_line` action, unbound by default |
| `^R` / `^X^R` | Hot reload (same as `/reload`). Clay keeps this on `^R`; TF's own `^R` is "refresh line", available as the unbound `refresh_line` action or `/dokey REFRESH` |
| `^X^V` | Show version (same as `/version`) |
| `^Z` | Suspend (resume with `fg` in your shell) |
| `Ctrl-C` (twice within 15 seconds) | Quit |
| `Enter` | Send the current input line |

## Popup Controls (All Popups)

| Key | Action |
|---|---|
| `Up` / `Down` | Navigate between fields |
| `Tab` / `Shift-Tab` | Cycle through buttons only |
| `Left` / `Right` | Navigate between buttons; change select/toggle values |
| `Enter` | Edit text field / toggle option / activate button |
| `Space` | Toggle boolean / cycle options |
| `Esc` | Close popup or cancel text edit |

**Button shortcuts:** the highlighted letter in a button's label (e.g.
**S**ave, **C**ancel, **D**elete) activates it directly, without needing to
tab to it first.

## Filter Popup (F4)

| Key | Action |
|---|---|
| Type text | Filter output to matching lines |
| `Backspace` / `Delete` | Edit filter text |
| `Left` / `Right` | Move cursor in filter text |
| `Home` / `End` | Jump to start/end of filter |
| `PageUp` / `PageDown` | Scroll through filtered results |
| `Esc` / `F4` | Close filter and restore the normal view |

## Help Popup (F1)

| Key | Action |
|---|---|
| `Up` / `Down` | Scroll one line |
| `PageUp` / `PageDown` | Scroll multiple lines |
| `O` | Highlight the Ok button |
| `Enter` / `Esc` | Close popup |

## World Selector (/worlds)

| Key | Action |
|---|---|
| `Up` / `Down` | Navigate the world list |
| `Enter` | Connect to the selected world / activate the focused button |
| `Tab` / `Shift-Tab` | Cycle: filter field → Add → Edit → Delete → Cancel → Connect |
| `A` | Add a new world |
| `E` | Edit the selected world |
| `D` | Delete the selected world |
| `O` | Connect to the selected world |
| `C` | Cancel/close |
| `F` | Focus the filter field |
| `Esc` | Close popup |

## Actions List (/actions)

| Key | Action |
|---|---|
| `Up` / `Down` | Navigate the action list |
| `Space` | Toggle enable/disable on the selected action |
| `Enter` | Edit the selected action |
| `Tab` | Cycle between the filter field, list, and buttons |
| `A` | Add a new action |
| `E` | Edit the selected action |
| `D` | Delete the selected action |
| `O` | Ok (close) |
| `F` or `/` | Focus the filter field |
| `Esc` | Close popup |

## Confirmation Dialogs

| Key | Action |
|---|---|
| `Left` / `Right` / `Up` / `Down` / `Tab` | Toggle between Yes/No |
| `Y` | Select Yes |
| `N` | Select No |
| `Enter` | Confirm the current selection |
| `Esc` | Cancel and close |

## Mouse (Console, when "Console Mouse" is enabled in /setup — default: on)

- Click popup buttons and fields to interact
- Click list items to select them
- Scroll wheel to scroll lists and scrollable content in popups
- Click and drag to highlight lines in scrollable content

---

## Appendix: Every Default Binding

The complete, definitive list — every entry `KeyBindings::defaults()`
(`src/keybindings.rs`) produces, sorted by key name. `cargo test
test_docs_key_table_matches_defaults` parses this exact table out of this
file and asserts it matches the code's own `PINNED_DEFAULTS` table in both
directions, so this appendix and the code cannot silently drift apart — if
you change a default, update both.

<!-- BEGIN DEFAULT KEY TABLE -->
| Key | Action id |
|---|---|
| `Alt-Down` | `input_shrink` |
| `Alt-Up` | `input_grow` |
| `Backspace` | `delete_backward` |
| `Ctrl-Down` | `history_next` |
| `Ctrl-End` | `recall_end` |
| `Ctrl-Home` | `recall_begin` |
| `Ctrl-Left` | `cursor_word_left` |
| `Ctrl-PageDown` | `flush_output` |
| `Ctrl-Right` | `cursor_word_right` |
| `Ctrl-Up` | `history_prev` |
| `Delete` | `delete_forward` |
| `Down` | `cursor_down` |
| `End` | `cursor_end` |
| `Esc--` | `kbnum_negative` |
| `Esc-.` | `insert_last_arg` |
| `Esc-0` | `kbnum_0` |
| `Esc-1` | `kbnum_1` |
| `Esc-2` | `kbnum_2` |
| `Esc-3` | `kbnum_3` |
| `Esc-4` | `kbnum_4` |
| `Esc-5` | `kbnum_5` |
| `Esc-6` | `kbnum_6` |
| `Esc-7` | `kbnum_7` |
| `Esc-8` | `kbnum_8` |
| `Esc-9` | `kbnum_9` |
| `Esc-<` | `recall_begin` |
| `Esc-=` | `goto_matching_bracket` |
| `Esc->` | `recall_end` |
| `Esc-Backspace` | `delete_word_backward_punct` |
| `Esc-J` | `selective_flush` |
| `Esc-L` | `toggle_limit` |
| `Esc-Left` | `world_socket_prev` |
| `Esc-Right` | `world_socket_next` |
| `Esc-Space` | `collapse_spaces` |
| `Esc-Tab` | `completion` |
| `Esc-^E` | `expand_line` |
| `Esc-^H` | `delete_word_backward_punct` |
| `Esc-^L` | `clear_screen` |
| `Esc-^N` | `scroll_line_forward` |
| `Esc-^P` | `scroll_line_back` |
| `Esc-_` | `insert_last_arg` |
| `Esc-b` | `cursor_word_left` |
| `Esc-c` | `capitalize_word` |
| `Esc-d` | `delete_word_forward` |
| `Esc-f` | `cursor_word_right` |
| `Esc-h` | `scroll_half_page` |
| `Esc-j` | `flush_output` |
| `Esc-l` | `lowercase_word` |
| `Esc-n` | `history_search_forward` |
| `Esc-p` | `history_search_backward` |
| `Esc-u` | `uppercase_word` |
| `Esc-v` | `toggle_insert` |
| `Esc-w` | `world_activity` |
| `Esc-{` | `world_prev` |
| `Esc-}` | `world_next` |
| `F1` | `help` |
| `F2` | `toggle_tags` |
| `F4` | `filter_popup` |
| `F5` | `search_popup` |
| `F8` | `toggle_action_highlight` |
| `F9` | `toggle_gmcp_media` |
| `Home` | `cursor_home` |
| `Insert` | `toggle_insert` |
| `Left` | `cursor_left` |
| `PageDown` | `scroll_page_down` |
| `PageUp` | `scroll_page_up` |
| `Right` | `cursor_right` |
| `Shift-Down` | `world_all_prev` |
| `Shift-Up` | `world_all_next` |
| `Tab` | `tab_key` |
| `Up` | `cursor_up` |
| `^A` | `cursor_home` |
| `^B` | `cursor_left` |
| `^D` | `delete_forward` |
| `^E` | `cursor_end` |
| `^F` | `cursor_right` |
| `^G` | `bell` |
| `^K` | `kill_to_end` |
| `^L` | `redraw` |
| `^N` | `history_next` |
| `^P` | `history_prev` |
| `^Q` | `spell_check` |
| `^R` | `reload` |
| `^S` | `pause_output` |
| `^T` | `transpose_chars` |
| `^U` | `kill_to_start` |
| `^V` | `literal_next` |
| `^W` | `delete_word_backward` |
| `^X[` | `scroll_half_page_back` |
| `^X]` | `scroll_half_page` |
| `^X^?` | `delete_word_backward_punct` |
| `^X^R` | `reload` |
| `^X^V` | `show_version` |
| `^X{` | `scroll_page_back` |
| `^X}` | `scroll_page_down` |
| `^Y` | `yank` |
| `^Z` | `suspend` |
| `^]` | `bg_all_worlds` |
<!-- END DEFAULT KEY TABLE -->

\newpage
