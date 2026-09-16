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

---

# Navigation rework (v1.6.12)

Three complaints after v1.6.11 shipped: the footer repeated the glyph the highlight
already marked, the highlight was too dark to see, and navigation was split across
three unrelated key groups (arrows → grid, Tab → category, typing → a search box with
no reachable caret).

## Progress checklist

- [x] **R1** — `start_edit` caret is a char index, not a byte index
- [x] **R2** — tab strip measures display columns, not chars
- [x] **R3** — `Category::tab_glyph()`
- [x] **R4** — grid cell styling: brighter, focus-aware, glyph-only
- [x] **R5** — drop `All`, glyph tabs, reorder fields, `Tabs.active`
- [x] **R6** — `refilter_emoji_console` + runtime title
- [x] **R7** — footer loses the glyph
- [x] **R8** — the focus model (the big one)
- [x] **R9** — help text
- [x] **R10** — wire: `emoji_categories_json`
- [x] **R11** — web/GUI
- [x] **R12** — verification
- [x] **R13** — web arrow parity and focus capture

**On resume:** find the first unchecked box, re-verify it landed, continue. Sequential.

## Approved design

Tab glyphs in `Category::all()` order: `😀 👋 🐶 🍕 ⚽ 💡 ⭐ 🏁`. All eight categories
stay. The `All` tab is removed in both interfaces; global search replaces it. The
active category name goes in the popup title (`┌ Emoji — Nature ──┐`).

**Mode is derived, never stored: browsing ⟺ the query is empty.** A non-empty query
means global results and no active tab.

### Key map — arrows only. Three zones: Search (row 0), Tabs (rows 1-2), Grid

| Key | Search | Tabs | Grid |
|---|---|---|---|
| printable / `Backspace` | edit query at caret | edit query (append/pop at end) | edit query (append/pop at end) |
| `↑` | — (top) | → Search | top row → Tabs; else up a row |
| `↓` | → Tabs | → Grid | down a row, clamps |
| `←` `→` | move the caret | prev/next category, **clears the query** | prev/next cell, **clamps at both ends** |
| `Home` `End` | caret to start/end | first/last category, clears the query | first/last cell |
| `PageUp` `PageDown` | scroll grid a page | scroll grid a page | scroll grid a page |
| `Enter` | insert selected grid cell, close | same | same |
| `Esc` | close | close | close |
| `Tab` / `Shift-Tab` | **nothing** | **nothing** | **nothing** |

**Movement is arrow keys only.** `Tab`/`Shift-Tab` are explicitly inert — they must not
fall through to anything else. Nothing here is reachable except by arrows, which is why
the popup carries no help text and no `?` button.

**Passing through the tab row does NOT clear the query.** Only a deliberate `←`/`→`/
`Home`/`End` *on* the tab row changes category, and only that clears the search. This
resolves the collision between global search and "landing on a tab shows that tab's
items": typing `heart` then pressing `↓↓` walks into the results with the query intact,
because merely traversing the strip changes nothing. While a query is live the strip is
drawn inactive (no underline), which telegraphs that no category is current.

You arrive on whatever `Tabs.selected_index` already held — the last browsed category,
Smileys at open. It is never reset.

Removed deliberately: `←`/`→` off the grid's first/last cell no longer step the
category (the tab row is directly reachable now), and `Tab` no longer changes category.

### Chrome the popup does NOT have

No footer line, no `?` help button, no help text, no blank spacer rows. The popup is the
title, the search box, the tab strip, and the grid — nothing else:

```
┌ Emoji — Objects ───────────────────────────────┐
│Search: ▏                                       │
│😀  👋  🐶  🍕  ⚽  💡  ⭐  🏁                  │
│                    ══                          │
│📱  💻  📷  📺  📻  💡  🔦  📖  📚  💵  💳  💎  │
│🔨  🔧  🔒  🔓  🔑  🔔  🎁  🎈  🎉  🎊  🏰  📜  │
│🔮  🧪  🪦  🪙  🧭  🏮  🪓  🏹  ⚓  🚢  🪄  💊  │
│💉  👓  📦                                      │
└────────────────────────────────────────────────┘
```

- **`EMOJI_FIELD_INFO` is deleted.** The selected emoji's name and keywords are shown
  nowhere: the highlight already says which emoji is selected and the title says which
  category. `emoji_info_text`/`update_emoji_info` go with it.
- **`.with_help(...)` is dropped**, removing the `?` button. Side effect worth having:
  that button is the one whose click makes the caller synthesize an `Enter`, which this
  popup reads as "insert the selected emoji and close" — so *clicking Help inserted an
  emoji*. Deleting the button deletes the bug for this popup.
- **The grid renders only as many rows as it has content**, capped at `visible_rows`, so
  a category that half-fills the grid leaves no trailing blank rows. This makes the
  popup's height change between categories and as a search narrows — the accepted cost
  of removing the empty lines.

## Traps — each of these was found by inspection, not by running the code

**`compute_tab_window` and `render_tabs_field` count `chars()`, not columns**
(`console_renderer.rs:1637` and the `prefix_width`/`active_label_width` arithmetic).
A glyph tab is 1 char and 2 columns, so with glyph labels the `═` underline lands at
`active * 3` instead of `active * 4` — **under the wrong tab**, drifting further with
each tab to the right. Must use `display_width()` (already defined at line 138)
regardless of anything else here.

**The highlight the user complained about is the UNFOCUSED one.** The popup opens with
focus on the search row, so the grid is unfocused in the state being complained about.
Brightening only the focused style leaves the bug in its commonest form. Both styles
change; the *difference* between them is what carries focus. Emoji are drawn in their
own colours by essentially every terminal, so `fg` on a selected cell is decorative —
**background is the whole signal.** Focused: `button_selected_bg()` (White/Blue, the
confirm dialog's focused-button colour). Unfocused: `fg_dim()` (DarkGray/Gray). Do
**not** use `fg_accent()` for unfocused — it is Blue in the Light theme and collides
with `button_selected_bg()`.

**The highlight must cover the glyph's 2 columns, not all 4 of `CELL_STRIDE`.** Today
one span covers the whole stride, so the highlight bleeds into the inter-cell gutter —
invisible at `Rgb(40,40,60)`, glaring at solid white. Split the cell into two spans.

**`?` stops being a search character** once `editing` can be false. The generic help
guard (`main.rs:15568-15580`) fires on `Char('?')` when `!state.editing`, which today
never happens here. Hoist `let popup_id` above that block and exclude the emoji popup,
or "typing always edits the search box" becomes false on two of the three rows.

**A live pre-existing bug this rework fixes for free:** `handle_popup_mouse_click`
(`input_handler.rs:98-147`) already calls `commit_edit()` when you click a different
field, setting `editing = false` — after which `insert_char` is a no-op and the search
box is dead until the popup is reopened. Any design that pins `editing = true` for the
popup's life is already broken by that code. The commit-on-focus-out /
start-edit-on-focus-in contract below is what that mouse path assumes.

**Do not touch `filter_emoji` or `getFilteredEmoji`.** "Global search" is
`filter_emoji(None, q)` — the same call that used to mean "the All tab". The meaning
changed; the code does not. The decision (`category = if query.is_empty() { tab } else
{ None }`) lives in the *callers* on both sides, which keeps the Rust/JS differential
check valid.

**Do not assert every tab glyph is an entry in its own category** — 🐶 (U+1F436) is not
in `EMOJI`; the table has 🐕 for `dog`. Assert `console_safe` + width 2 + all-distinct.

**`render_popup_content_direct` needs no signature change** — `let is_selected =
matches!(&state.selected, ElementSelection::Field(id) if *id == field.id);` inside its
existing loop. It needs no `Tabs` arm either: it only runs on mouse-wheel scroll, which
cannot change the active tab. Only the Grid styling must stay byte-identical with the
ratatui path, or the highlight flickers between two styles on partial repaints.

**The web keeps its own input model.** It adopts the glyph tabs, the dropped `All` and
global search — and nothing else. A browser search box has a native text caret; taking
`←`/`→` away from it to drive zone focus is a regression a mouse-and-keyboard user
feels immediately. Web `Tab`/`Shift-Tab` stay prev/next category, arrows stay on the
grid.

## The search text, now that focus moves

The `EMOJI_FIELD_SEARCH` Text field's own `value` is the single source of truth.
`edit_buffer`/`edit_cursor` are a working copy that exists only while the search row
has focus: `start_edit()` on focus-in, `commit_edit()` on focus-out. Typing while focus
is elsewhere appends directly to the field's `value` (at the end — with no visible
caret, end-of-string is the only defensible position).

- `emoji_focus(state, zone)` is the only thing that moves focus: commit, select, and
  `start_edit()` iff the target is Search.
- `emoji_query_insert`/`emoji_query_backspace` delegate to `state.insert_char`/
  `backspace` when Search is focused-and-editing, else mutate the field value directly.
- `emoji_query(state)` is the read side — character-for-character the expression
  already duplicated at `input_handler.rs:1032` and `remote_client.rs:1984`, so the
  refilter contract does not change, it just stops being copy-pasted.

`start_edit` sets `edit_cursor` to `edit_buffer.len()` — a **byte** count — while
`insert_char`/`backspace` treat it as a char index (`popup/mod.rs:1508`). Harmless only
because the popup opens empty; re-entering the search row with text in it makes it
reachable. Fix to `chars().count()` (R1) — a strict improvement for every popup.

## Killing the duplicated refilter

`EmojiFilter` is handled by ~25 verbatim-duplicated lines in `input_handler.rs` and
`remote_client.rs`. Extract `refilter_emoji_console(state)` doing grid → footer → title
→ `Tabs.active` in one pass; both call sites become one line, and `open_emoji_popup`
uses it too so the open path and the keypress path cannot diverge. `EmojiFilter` keeps
its empty payload — the `selected_index` re-read moves inside the helper.

Pin it with an `include_str!` test asserting both files mention
`refilter_emoji_console` — crude, but it is the only thing that catches "someone added
a third console front end and forgot the arm", which is the failure mode this file
already warns about for `remote_client.rs`.

## Known adjacent breakage — NOT in scope, recorded so it is not rediscovered

- Clicking a tab only moves focus; it does not change the active tab or refilter.
- Clicking a grid cell does not select it (`grid_select` has no production caller).
- `handle_paste` writes `edit_buffer` and returns without reaching `EmojiFilter`, so a
  pasted search term never refilters.

# R13 — web arrow parity and focus capture

Reported: arrow keys in the web/GUI picker moved nothing visible, and the emoji popup
lost the keyboard back to the command input while open. Root cause was two focus-steal
paths plus the web picker never having adopted the arrows-only zone model at all (the
"web keeps its own input model" decision from the Navigation rework above is superseded
by this entry — the web now mirrors the console's key map exactly).

**Focus capture (`src/web/app.js`):**
- `elements.input`'s own `keydown` listener now returns immediately
  (`if (emojiPopupOpen) return;`) before resolving any chord/action, so it can never
  intercept-and-`stopPropagation()` a key meant for the popup.
- The document-level nav-dispatch block's two `elements.input.focus()` calls (the
  `tf_bound_keys_json`/`RunKeyBinding` arm and the built-in-action-table arm) are now
  guarded with `if (!emojiPopupOpen)` — opening the popup from that path (chord
  resolved while nothing had DOM focus) no longer steals focus back to the command
  input on the very next line.
- `openEmojiPopup`/`closeEmojiPopup` are unchanged in effect (search gets focus on
  open, `focusInputWithKeyboard()` restores it on close) but DOM focus now genuinely
  stays on `elements.emojiSearch` for the popup's entire lifetime — see below.

**Zone model:** a new `emojiZone` variable (`'search' | 'tabs' | 'grid'`), moved by
`emojiSetZone(zone)`, which just sets `data-zone` on `elements.emojiModal` — no
re-render needed, since all the visual difference is CSS. Unlike the console, DOM focus
never actually leaves `elements.emojiSearch`; the zone is a pure UI-state indicator.
This is what keeps the search box's native caret working while still letting Up/Down
walk into Tabs/Grid.

New functions in `app.js`, each a direct port of the `PopupState`/`emoji.rs` method it
mirrors: `emojiGridMove(dx, dy)` (`grid_move`), `emojiGridCursor()` (`grid_cursor`),
`emojiGridPage(delta)` (`grid_page`), `emojiTabsSelectEdge(last)`
(`emoji_tabs_select_edge`, via the existing `setEmojiCategory`). The old flat-clamp
`emojiStep(delta)` is gone, replaced by `emojiGridMove(dx, 0)`, which is the same
behavior for `dx = ±1` but is also now dy-aware for real row movement.

The `document.onkeydown` `emojiPopupOpen` block was rewritten key-by-key against the
key-map table: `←`/`→`/`Home`/`End` return without `preventDefault()` while
`emojiZone === 'search'` (native caret), and are intercepted everywhere else; `Tab`/
`Shift-Tab` are unconditionally `preventDefault()`-ed and otherwise ignored in every
zone; `PageUp`/`PageDown` call `emojiGridPage` regardless of zone; printable characters
and `Backspace` are never intercepted at all (typing always lands in the search box
natively and never changes zone or clears the query — only a deliberate `←`/`→`/
`Home`/`End` *on* the Tabs zone does that, via the existing `setEmojiCategory`/
`emojiTabsSelectEdge`).

**CSS (`src/web/style.css`), all keyed off `#emoji-modal[data-zone="…"]`:**
- `.emoji-grid button.emoji-cell.selected` is now the *unfocused* look
  (`var(--theme-selection-bg)`, a muted blue in both the light and dark palettes, on
  `var(--text-color)`); `#emoji-modal[data-zone="grid"] .emoji-cell.selected` restores
  today's bright `var(--accent-color)`-on-black look, matching the console's
  `button_selected_bg()` vs. `fg_dim()` split.
- `#emoji-modal[data-zone="tabs"] .emoji-tab-btn.active` gets the same bright
  accent-color fill while the Tabs zone actually has the arrow keys; the plain
  underline (`.emoji-tab-btn.active` alone) still marks "current category" the rest of
  the time.
- `.emoji-searchrow input:focus` no longer shows the accent border unconditionally
  (DOM `:focus` never leaves this input while the popup is open, so that rule used to
  be permanently "on" and stopped meaning anything) — it now defaults to the plain
  separator border, and `#emoji-modal[data-zone="search"] .emoji-searchrow input:focus`
  is what lights it up.

**Verification:** no node in this sandbox. Served `src/web/` via `python3 -m
http.server` on 127.0.0.1 and drove real headless Firefox through geckodriver's HTTP
API (`-headless`, per `reference_clay_js_browser_harness`). `new Function(src)` on the
live-fetched `app.js` confirmed no `SyntaxError`. A harness fetched the real file text,
extracted (via brace-matching, not reimplementation) the actual `emojiGridMove`/
`emojiGridCursor`/`emojiGridPage`/`emojiTabsSelectEdge`/`emojiSetZone`/
`getEmojiGridMetrics`/`setEmojiCategory` functions and the real `emojiPopupOpen`
keydown block, wired them against a real (off-screen, fixed 4-column) grid element so
`getEmojiGridMetrics`'s DOM measurement ran for real, and ran 20 assertions: Search→
Tabs→Grid on repeated `↓`, Grid-top-row `↑`→Tabs, `←`/`→` clamp at both grid ends,
`←`/`→`/`Home`/`End` are not `preventDefault`-ed in Search but are everywhere else,
typing `heart` then `↓↓` preserves the query while reaching Grid, `Tab` is inert, and a
plain character key is never intercepted — all 20 passed. A second harness rendered a
real `.emoji-cell.selected` and `.emoji-tab-btn.active` under the real `style.css` and
toggled `data-zone`: focused-vs-unfocused `background-color` differed for both the grid
cell and the tab (confirmed again with the actual dark/light `--theme-selection-bg` hex
values injected, for real contrast against `--text-color` in both themes), and the
search input's border color differed between `data-zone="search"` and the other two
zones despite DOM `:focus` never moving. `cargo test --no-default-features --features
rustls-backend,ssh-transport`: 1778 passed, 0 failed (matches baseline). All spawned
geckodriver/http.server processes were killed after verification; the user's own
Firefox windows were left untouched.
