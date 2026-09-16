# Emoji Picker Roadmap (`Esc-e`)

Self-contained implementation record for the emoji picker popup. Everything an
implementer needs is in this file — do not go looking for the planning session.

## Progress checklist

- [x] **Step 1** — `src/emoji.rs`: curated table, `Category`, `emoji_json()`, and
      `encoding::emoji_name_to_unicode` converted to a lookup over it
- [x] **Step 2** — `FieldKind::Tabs` + `FieldKind::Grid` in `src/popup/mod.rs` and
      their renderers/measurers in `src/popup/console_renderer.rs`
- [x] **Step 3** — `src/popup/definitions/emoji.rs`: `create_emoji_popup`,
      `filter_emoji`, `update_emoji_grid`
- [x] **Step 4** — console wiring: `NewPopupAction::{EmojiFilter, InsertText}`, the
      `is_emoji` key block, `App::open_emoji_popup`, and the two result-match sites
- [x] **Step 5** — action id registration (`emoji_picker`) + the `Esc-e` default +
      every test fixture and doc table that pins the default set
- [x] **Step 6** — wire: `emoji_json` on `InitialState`
- [x] **Step 7** — web/GUI: `index.html`, `style.css`, `app.js`
- [x] **Step 8** — full verification pass

**On resume:** find the first unchecked box above, re-verify that step actually
landed (build + its own tests), then continue from there. Steps are sequential —
never run two at once.

---

## What is being built

A modal popup, opened with `Esc-e`, in all three interfaces (console TUI, web,
WebView GUI). It has:

- a **tab strip** of emoji categories across the top (`All`, then 8 categories),
  active tab in the accent colour with an underline beneath it;
- a **search box** that narrows within the active tab;
- a **grid** of emoji with a 2-D cursor;
- a **footer** showing the selected emoji, its `:shortcode:` and its keywords.

`Enter` (or a click) inserts the **emoji character** at the cursor in the command
input and closes the popup. `Esc` closes without inserting.

Approved mockup: <https://claude.ai/artifact/QwjJ4v2bxALbfq1WyM9DVj>

### Console layout (target)

```
┌ Emoji ─────────────────────────────────────────┐
│ ‹ All  Smileys  People  Nature  Food  Activi › │
│        ═══════                                 │
│ Search: hear▏                                  │
│                                                │
│   😍  🥰  😘  💕  💖  💗  💓  💞  💘  ❤️       │
│   🧡  💛  💚  💙  💜  🖤  🤍  🤎  💔  ❣️       │
│  [💝] 💟  💌  😻  🫀  🥹  🤗  🙌  🫶  👐       │
│   🤝  💐  🌹  🌺  🌸  🌷  🥀  🌻  🌼  🏵       │
│                                                │
│ 💝  :gift_heart:   heart, gift, love, valentine│
│ Tab category · ↑↓←→ move · Enter insert · Esc  │
└────────────────────────────────────────────────┘
```

### Key map (identical in all three UIs)

| Key | Action |
|---|---|
| `Esc-e` | open the picker |
| printable char | append to search |
| `Backspace` | delete from search |
| `←` `→` `↑` `↓` | move the grid cursor |
| `←` on the first cell / `→` on the last | step to the previous / next category |
| `Tab` / `Shift-Tab` | next / previous category |
| `PageUp` / `PageDown` | scroll the grid a page |
| `Home` / `End` | first / last cell |
| `Enter` | insert the emoji, close |
| `Esc` | close without inserting |

### Decisions already taken — do not relitigate

- `Esc-e` **only**. No `/emoji` command, no hamburger-menu row, no icon-bar tile,
  no `/menu` entry. On Android the picker therefore needs a physical keyboard;
  that is accepted.
- Curated set of ~400 emoji, **not** the full Unicode set.
- Insertion is the **emoji character**, never `:shortcode:`.
- `All` is the first tab and the default, so search covers everything by default.
- Out of scope: skin-tone modifiers, a recently-used list, per-world defaults,
  anything persisted to `settings.dat`. The picker is stateless between openings.

---

## Step 1 — `src/emoji.rs`

Create `src/emoji.rs` and register it in `src/main.rs`'s module list.

```rust
pub enum Category { Smileys, People, Nature, Food, Activity, Objects, Symbols, Flags }

pub struct EmojiEntry {
    pub ch: &'static str,
    pub name: &'static str,          // the :shortcode: body, unique across the table
    pub aliases: &'static [&'static str],   // extra search keywords
    pub category: Category,
}

pub const EMOJI: &[EmojiEntry] = &[ /* ~400 entries */ ];
```

`Category` needs `all()` (iteration order for the tab strip), `label()` (the tab
text) and a stable `index()`.

**Seed the table from the existing data.** `encoding.rs`'s `emoji_name_to_unicode`
(currently around lines 1256–1535) is a private 254-arm `match` whose `|`-separated
alias lists are exactly the keywords the picker needs, already grouped by
`// Smileys & Emotion`-style section comments that map onto the eight categories.
Move that data here — first arm name becomes `name`, the rest become `aliases` —
then grow it to ~400 entries with the obvious gaps (more food, more fantasy/MUD
items, flags).

Then **reimplement** `encoding::emoji_name_to_unicode` as a lookup over a
`OnceLock<HashMap<&'static str, &'static str>>` built from `EMOJI` (name + every
alias → `ch`). Keep its existing signature, its `to_lowercase()` on the input, and
its `Option<String>` return. Its only caller is `convert_discord_emojis`
(`encoding.rs`, ~line 1217); `convert_discord_emojis_with_links` is the sibling
that `rendering.rs` actually calls and is unaffected.

**Hard constraint — every entry must render exactly 2 terminal cells.** The console
grid pads to a fixed stride instead of measuring, because `unicode_width` reports 1
for VS16 sequences that terminals draw as 2 (see the `ARCHIVE_PREFIX_WIDTH` comment
in `src/util.rs`, ~line 217). So exclude narrow glyphs: `©`, `™`, `▪`, bare arrows,
and anything else whose emoji presentation is not full-width. Regional-indicator
flag pairs and ZWJ sequences are fine — they are two cells.

Add `pub fn emoji_json() -> String` producing the compact wire form
`[[ch, name, "kw1 kw2", category_index], …]` (used in Step 6). Build it once into a
`OnceLock<String>`.

**Tests (in `src/emoji.rs`):**
1. every entry's display width is 2 — count chars, treat any multi-char grapheme
   (VS16, ZWJ, regional pair) as 2, otherwise `UnicodeWidthChar::width`;
2. `name` is unique across the table, and no alias collides with another entry's
   name or aliases;
3. a sample of ~25 aliases that resolved through the old `match` still resolve to
   the same emoji through the new lookup (behaviour preservation);
4. `emoji_json()` parses as JSON and has one row per entry.

---

## Step 2 — two new popup field kinds

### `src/popup/mod.rs`

Add to `FieldKind` (the enum starts around line 61):

```rust
/// Horizontal tab strip. Two rendered rows: the labels, then the active
/// tab's `═` underline. Scrolls horizontally when wider than the popup.
Tabs { labels: Vec<String>, selected_index: usize, scroll_offset: usize },

/// Grid of fixed-stride cells with a 2-D cursor. Reuses `ListItem`:
/// `columns` = [display, name, keywords], `id` = the text to insert.
Grid { cells: Vec<ListItem>, selected_index: usize, scroll_offset: usize,
       columns: usize, visible_rows: usize },
```

Reusing `ListItem` (defined ~line 127) keeps `id` as the return value, matching
every other selection popup. Add constructors alongside the existing ones
(`list_with_headers_and_widths` etc., ~lines 141–291).

Arms/updates needed in this file:

- `FieldKind::is_interactive` (~line 417) — both are interactive.
- `FieldKind::is_text_editable` (~line 422) — neither is.
- `PopupState::ordered_elements` (~line 766) — both are focusable.
- Navigation helpers mirroring `list_select_up`/`list_select_down` (~lines 1931,
  1948): `grid_move(dx, dy)`, `grid_select(index)`, `grid_page(delta)`,
  `grid_home()`/`grid_end()`, `tabs_step(delta)`, plus a
  `get_selected_grid_item() -> Option<&ListItem>`. `grid_move` must **not** wrap
  within the grid — stepping off the first/last cell is reported to the caller so
  it can change category instead (return `bool`/an enum saying "hit the edge").
- `PopupState::next_item`/`prev_item` (~lines 1368, 1392) stay inside a `Grid`
  the same way they stay inside a `List`.

### `src/popup/console_renderer.rs`

Arms needed:

- `calculate_content_width` (~line 136): `Tabs` → longest fitting window, but let
  `layout.min_width` win; `Grid` → `columns * CELL_STRIDE + 2`.
- `calculate_content_height` (~line 182): `Tabs` → 2; `Grid` → `visible_rows`.
- `render_field` (~line 457): two new private renderers, below.
- `render_popup_content_direct` (~line 1530, the crossterm fast-scroll path):
  add a `Grid` arm, or the grid silently loses smooth scrolling.

`render_grid_field`: **build each row as explicit `Span`s, never one
`format!("{:<w$}")` string.** `render_list_field` (~line 896) is the anti-pattern
here — it measures with `.len()` (bytes) and pads by char count, both wrong for
emoji. Use a `const CELL_STRIDE: usize = 4` (2 glyph cells + 2 gutter) and a
private `emoji_cell_width()` that returns 2, with a `debug_assert!` tying it to the
Step 1 table test. The selected cell carries the popup selection style
(`fg_accent()` on `selection_bg()`); everything else is plain.

`render_tabs_field`: window the label list so the active tab is always visible,
grow the window outward from the active tab until it no longer fits, and draw `‹`
/ `›` in the dim colour for the hidden ends. Row 2 is `═` repeated under the active
label only, in the accent colour. Keep `scroll_offset` on the field so the window
is stable across repaints.

`truncate_str` (~line 1486) counts chars, not columns — do not use it on a row that
contains emoji.

**Tests:** extend the existing `compute_field_layout` unit tests (that function,
~line 251, is pure and already covered) with a popup containing both new kinds, and
add direct tests for `grid_move` edge reporting, `grid_page` clamping, and the tab
windowing function.

---

## Step 3 — `src/popup/definitions/emoji.rs`

Model directly on `src/popup/definitions/world_selector.rs` (~line 35), the closest
existing popup — search field plus a live-narrowed list.

```rust
pub const EMOJI_FIELD_TABS:   FieldId = FieldId(1);
pub const EMOJI_FIELD_SEARCH: FieldId = FieldId(2);
pub const EMOJI_FIELD_GRID:   FieldId = FieldId(3);
pub const EMOJI_FIELD_INFO:   FieldId = FieldId(4);   // footer Label

pub fn create_emoji_popup(visible_rows: usize, columns: usize) -> PopupDefinition;
pub fn filter_emoji(category: Option<Category>, query: &str) -> Vec<ListItem>;
pub fn update_emoji_grid(state: &mut PopupState, cells: &[ListItem]);
pub fn update_emoji_info(state: &mut PopupState);   // refresh the footer Label
```

- The search field gets `.search()` so `ordered_elements` pins it first, exactly as
  the world selector's filter does.
- `filter_emoji`: `category == None` means the `All` tab. Matching is a
  **case-insensitive substring** over `name` + aliases — the same
  `RecallMatchStyle::Simple` rule the rest of Clay's search uses, never whole-word.
- `update_emoji_grid` mirrors `update_world_list` (`world_selector.rs` ~line 147),
  including clamping `selected_index` and `scroll_offset` after a refilter.
- Register with `pub mod emoji;` + re-export in `src/popup/definitions/mod.rs`.

**Tests:** `filter_emoji` finds by name, by alias, is case-insensitive, respects the
category filter, returns everything for `All` with an empty query, and returns an
empty vec for a nonsense query.

---

## Step 4 — console wiring

**`src/main.rs`:**

1. Two new `NewPopupAction` variants (enum ~line 15258):
   ```rust
   EmojiFilter,          // search text or category changed — caller refilters
   InsertText(String),   // Enter — insert at the input cursor
   ```
2. `let is_emoji = popup_id == Some(popup::PopupId("emoji"));` alongside the other
   `is_*` flags (~line 15544).
3. An `if is_emoji { … }` block following the `is_world_selector` block's shape
   (~lines 15570–15615). It owns **every** key while open — a popup swallows input
   before any keybinding action runs (`input_handler.rs` ~line 446). Implement the
   key map table above. Printable chars and Backspace edit the search box and
   return `EmojiFilter`; category changes return `EmojiFilter`; Enter returns
   `InsertText(selected.id)` and closes; Esc closes.
4. `App::open_emoji_popup()` next to `open_world_selector_new` (~line 5705):
   derive `columns`/`visible_rows` from the terminal size, build the definition,
   open it, select the search field, `start_edit()`.

**`src/input_handler.rs`** (~line 447, where `NewPopupAction` is matched):

- `EmojiFilter` → read the search text (`state.edit_buffer` while editing, else
  `state.get_text(EMOJI_FIELD_SEARCH)`), call `filter_emoji` + `update_emoji_grid`
  + `update_emoji_info`. Copy the `WorldSelectorFilter` arm (~lines 593–618).
- `InsertText(s)` → `app.input.insert_str(&s)` (`src/input.rs` ~line 249) and close
  the popup. `insert_str` is byte-offset based and already correct for multi-byte
  text — it is what `handle_paste` uses.

**`src/remote_client.rs`** (~line 1483): the SSH remote console shares
`handle_new_popup_key` but keeps its **own** `NewPopupAction` match. Add the same
two arms there. Nothing tests this file's match — a missing arm is a silently dead
popup over SSH.

---

## Step 5 — action registration

Action: `id: "emoji_picker"`, `name: "Emoji Picker (Esc-e)"`, `category: "Clay"`.
Seven sites, three of them test-enforced:

| # | Site | Note |
|---|---|---|
| 1 | `ACTIONS` — `src/keybindings.rs` ~line 150 | with the other `"Clay"` entries |
| 2 | `defaults()` — `src/keybindings.rs` ~line 326 | `b.insert("Esc-e".into(), "emoji_picker".into())`. **No** `Alt-e` mirror — the `meta_aliases` pass (~lines 352–373) covers named keys only, and `Alt-e`/`Esc-e` are the same bytes on the wire |
| 3 | `PINNED_DEFAULTS` — `src/keybindings.rs` ~line 837 | test fixture; `test_default_table_pinned` asserts two-way set equality |
| 4 | `dispatch_action_impl` — `src/input_handler.rs` ~line 1599 | `test_dispatch_action_handles_every_action_id` (`src/tests.rs` ~line 14944) fails without it |
| 5 | `dispatchActionImpl` — `src/web/app.js` ~line 11909 | Step 7; no test covers JS dispatch |
| 6 | `dispatch_remote_action` — `src/remote_client.rs` ~line 2289 | SSH console's own copy; untested |
| 7 | `keybind-editor.html` `order` array ~line 1093 | **no change** — `"Clay"` is already listed |

Docs — `test_docs_key_table_matches_defaults` (`src/keybindings.rs` ~line 1223)
parses two of these and requires rows **sorted by key**, so `Esc-e` goes between
`Esc-d` and `Esc-f`:

- `TINYFUGUE-COMPAT.md` appendix table (~line 314) — **checked**
- `docs/markdown/07-keyboard-shortcuts.md` appendix table (~line 319) — **checked**
- `docs/markdown/07-keyboard-shortcuts.md` General section (~line 158) — prose row
- `TINYFUGUE-COMPAT.md` "Clay extras" ruling row (~line 66) — `Esc-e` is a Clay
  addition, not a TF default
- `src/tf/parser.rs` `/help keys`, Display section (~line 2340) — not checked; uses
  `Esc+E` plus-sign notation
- `src/popup/definitions/help.rs` — one line in the F1 help text

---

## Step 6 — wire protocol

Add `emoji_json: String` as a **top-level field on `WsMessage::InitialState`** in
`src/websocket.rs` — deliberately **not** on `GlobalSettingsMsg`, which is
re-broadcast on every settings change. The table is static, so once per connection
is enough; the compact form keeps it near 16 KB.

- `#[serde(default)]` on the field is **mandatory**. Without it a new client against
  an old server fails to deserialize the whole message — the `WS-BAD-MESSAGE` class
  CLAUDE.md warns about.
- Populate it from `emoji::emoji_json()` in `build_initial_state` (`src/main.rs`)
  and anywhere else `InitialState` is constructed (check `src/daemon.rs` and
  `src/remote_client.rs`).
- Client side (Step 7): an absent or empty `emoji_json` must degrade to "the picker
  opens with a one-line notice", never a throw.

---

## Step 7 — web / GUI

One implementation: `src/webview_gui.rs` loads the same web app, and
`android/app/build.gradle` (the `copyWebAssets` task) copies `src/web/*` into the
APK at build time.

**`src/web/index.html`** — new `<div id="emoji-modal" class="modal">` next to
`actions-list-modal` (~line 316): `.popup-header` with title + close button, a
`.emoji-tabs` row of `<button class="emoji-tab-btn" data-cat="…">`, a search
`<input>`, a scrolling `#emoji-grid`, and an `.emoji-info` footer.

**`src/web/style.css`** — model the shell on `.settings-modal-content` (~line 2189,
the newest modal styling) and the tab buttons on `.settings-tab-btn` (~line 2217),
but laid out horizontally rather than as the vertical sidebar that rule assumes.
Selected cell uses the house convention `background: var(--accent-color); color:
#000`. The grid is `display: grid; grid-template-columns: repeat(auto-fill,
minmax(38px, 1fr))` with **`align-content: start`** (without it the rows stretch to
fill the fixed-height container and the grid looks broken). This is the **first CSS
Grid in the codebase** — everything else is flexbox — so keep it contained here.
The tab row scrolls horizontally with `scrollbar-width: none` so it does not draw a
band across the panel.

**`src/web/app.js`** —

- element refs in the `elements` object (~lines 505–560);
- `openEmojiPopup()` / `closeEmojiPopup()` / `renderEmojiGrid()` /
  `getFilteredEmoji()`, modelled on `openActionsListPopup` (~line 9516) and
  `renderActionsList` (~line 9571) — including the latter's `maxHeight` computation
  from `window.innerHeight` minus chrome, so the grid fits on a phone;
- `getFilteredEmoji` must agree with the Rust `filter_emoji` exactly (same
  case-insensitive substring rule over name + keywords);
- `case 'emoji_picker':` in `dispatchActionImpl` (~line 11909), two lines like
  `case 'filter_popup'` (~line 12387);
- an `if (emojiPopupOpen) { … return; }` block in the `document.onkeydown` chain
  (~line 13575), with the other popup blocks, implementing the key map;
- an entry in `closeAllPopups` (~line 12703);
- insertion reuses the `lastArgument()` idiom (~lines 6246–6257): capture
  `elements.input.selectionStart` **before** the modal takes focus, splice on
  insert, then call `resetCompletion()` and refocus via `focusInputWithKeyboard()`
  (~line 12690) so the Android soft keyboard comes back. Setting `.value` fires no
  `input` event, so the follow-ups must be called by hand — same as the
  overwrite-mode path (~lines 13941–13943);
- a `popupHelpTexts` entry (~line 7050) for the shared `?` overlay.

**Not needed:** the hardcoded global-shortcut allow-list (~line 13607) only sees
non-chord keys via `keyEventToName`, which cannot produce `Esc-e`. Add
`emoji_picker` there only if the binding ever moves to an F-key.

---

## Step 8 — verification

1. `cargo test` — all green, including the new Step 1/2/3 tests and the four
   existing tests that fail until Step 5 lands.
2. `cargo clippy` — clean.
3. `cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend,ssh-transport`
4. **Console, for real:** drive the built binary in a pty and model the painted
   screen. Assert the grid rows are column-aligned — every cell starts at the
   expected column, which is the entire point of the dedicated renderer — that the
   tab underline sits under the active tab, and that `Esc-e` → arrows → `Enter`
   leaves the right emoji in the input buffer.
5. **Web/GUI JS:** whole-file syntax check, plus running the extracted
   `getFilteredEmoji` and grid-navigation functions in headless Firefox via
   geckodriver against the same fixtures the Rust `filter_emoji` tests use — the two
   must agree.
6. **Manual:** `clay --gui`, `Esc-e`, search, click, confirm insertion lands at the
   cursor (not at the end) with text either side of it.

---

## Known adjacent bug (NOT in scope, do not fix here)

`^]` → `bg_all_worlds` (`src/keybindings.rs` ~line 308) can never fire in the
console: crossterm 0.27 decodes `Ctrl+]` (0x1D) as `Char('5')` + CONTROL, so the
console names that key `^5` while the browser names it `^]`. The binding works in
the web UI only. Recorded here so it is not rediscovered as a side effect of this
work.

---

## Post-release fixes

### Web: tab strip unusable (reported 2026-09-15)

Only the `All` tab was visible, at full panel width, with no way to reach the
other eight.

Two stacked causes:

1. `style.css`'s `.modal-content button:not(.popup-close):not(.stepper-btn):not(.btn)`
   is specificity (0,4,1) and sets `width: 100%`. `.emoji-tab-btn` is (0,1,0) and
   lost, so every tab became a full-width row. The same rule was also giving the
   grid cells a `margin-top` and an opaque background that should only appear on
   hover. Fixed by adding `:not(.emoji-tab-btn):not(.emoji-cell)` — the
   opt-out-by-name pattern that selector already used three times.
2. Nine tabs need ~560px; the panel is 420px, and the strip scrolled sideways
   with `scrollbar-width: none`, so five categories sat behind a deliberately
   invisible scrollbar. Fixed by letting the strip **wrap** to a second line. The
   console picker scrolls its strip with chevrons because a terminal row cannot
   wrap; a browser has no such constraint.

### Console: grid slid into the border and scrollbar (reported 2026-09-15)

Root cause: **`unicode_width` answering 2 is not a promise that the terminal
draws 2.** The crate applies emoji presentation and answers 2 for all 389
entries, and ratatui lays its buffer out with those numbers — but a terminal that
ignores a VS16 selector draws `❤️` in one column, and one that cannot compose a
ZWJ sequence draws `❤️‍🔥` as two glyphs in four. Either way the row slides
relative to the buffer and its last cell lands on the popup's own border.

Note this is the **opposite** of the hazard `util.rs`'s `ARCHIVE_PREFIX_WIDTH`
comment describes; do not assume one implies the other.

Fixes:

- `emoji::console_safe(ch)` — rejects any glyph containing `U+FE0F` or `U+200D`,
  then requires `unicode_width == 2`. Computed, never a hand-maintained list.
  Regional-indicator flag pairs are deliberately **kept**: composed into one flag
  or drawn as two boxed letters, they occupy 2 columns either way, so there is
  nothing to drift and the Flags tab is not left empty.
- `filter_emoji_console` in `popup/definitions/emoji.rs` wraps `filter_emoji`
  with that predicate, and the three console call sites (`main.rs`,
  `input_handler.rs`, `remote_client.rs`) use it. Deliberately NOT folded into
  `filter_emoji`, which is the shared matching rule that `getFilteredEmoji` in
  app.js mirrors and is cross-checked against — this is a rendering limit, not a
  change to what "matches". 351 of 389 entries reach the console grid; the
  web/GUI picker and `:shortcode:` lookup still see all 389.
- `console_renderer::calculate_content_width` measured `Label`/`ErrorText` in
  **bytes** (`str::len()`). Identical for the ASCII every other popup uses, but
  the emoji footer leads with a 4-byte 2-column glyph. Now uses display width.

### Console: `:shortcode:` removed from the footer

The footer showed `😊  :smile:   smiley, happy`. It now shows the glyph and its
search terms with the name leading the list — `⚽   soccer, football` — with no
colon form anywhere. Pinned by `test_emoji_info_text_has_no_shortcode_form`.
