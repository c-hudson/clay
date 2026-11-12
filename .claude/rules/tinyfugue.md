---
paths:
  - "src/tf/**"
  - "src/tfrun.rs"
  - "src/tfstartup.rs"
  - "src/keynames.rs"
  - "src/chords.rs"
  - "src/keybindings.rs"
  - "tests/tf/**"
---
# TinyFugue compatibility

Design record for the TF engine, its script-test suite, the key-name grammar and dispatch order, the status line. Claude Code loads this file when one of the paths above is read or edited; CLAUDE.md ("Rules That Bite Everywhere") keeps each rule as one sentence with a pointer here. The text below is the full detail, history and function names, moved verbatim from CLAUDE.md on 2026-11-08. Complements TINYFUGUE-COMPAT.md (design record) and reference/tf-engine.md.

Clay's TinyFugue (TF) compatibility layer (`src/tf/`) and its keybinding grammar
(`src/keynames.rs`, `src/chords.rs`, `src/keybindings.rs`) are specced against real TF
5.0 in `TINYFUGUE-COMPAT.md` (design record — per-key and per-command ruling tables,
changed defaults, intentional differences) and `docs/markdown/06-tf-commands.md` /
`docs/markdown/07-keyboard-shortcuts.md` (user-facing).

- **Script-test suite**: `tests/tf/cases/*.tf` + `.expected`, run via `cargo test
  tf_script` (harness: `src/tf/script_tests.rs`). Add a fixture by writing the `.tf`
  file, then run `tools/tf-oracle.sh --write <name>.tf` to generate its `.expected`
  from real `tf` (needs `tf` on `PATH` — `apt install tf5`) — never hand-write an
  `.expected` file. A case needing the real TF library starts with `;; requires-lib`
  and is skipped (not failed) when `$TFLIBDIR`/`/usr/share/tf5/tf-lib` isn't found.
- **The xfail ledger** (`tests/tf/xfail.txt`, format `case-name | substring-of-failure`):
  a known-failing case is listed here, not silently left red. A listed case that
  starts passing is itself a test failure ("remove it from xfail.txt") — that's the
  signal a fix landed. Never add an entry to make a red test green without a real,
  understood gap behind it.
- **Key-name grammar and dispatch order**: every key name (`keybindings.dat`, `/bind`,
  a live keypress, the web keybind editor) is canonicalized through
  `crate::keynames::parse_key_name`/`KeySeq::canonical` — this is the one place chords
  (`^X^R`), TF's raw spellings (`^[b`, `\033`), and case-significant `Esc-` forms
  (`Esc-j` != `Esc-J`) are understood. A pressed key resolves in this order, identical
  on console and web/GUI/SSH-console clients: (1) a `/bind`/`/def -b`/`-B` match, (2) a
  `key_<name>` macro, (3) the built-in action table (`ACTIONS`/`dispatch_action`), (4)
  literal character input.
- **Three-UI rule for a new action id**: adding to `ACTIONS` in `src/keybindings.rs`
  is not enough on its own — `dispatch_action` (console), `dispatchActionImpl` in
  `app.js` (web/GUI), `dispatch_remote_action` in `src/remote_client.rs` (the SSH
  remote console keeps its **own** copy of the dispatcher, so this is a fourth site,
  not covered by the other three — an action missing here does nothing at all when the
  key is pressed over SSH), and the keybind editor's action list (it reads
  `actions_json`, but a new category string also needs adding to its own `order` array
  or the whole category is silently hidden) all need the new id, or an interface
  silently can't run it. `RunKeyBinding{key, kbnum}` is the wire message a remote client sends back to the
  server when a keypress matches a TF `/bind` entry the server told it about
  (`tf_bound_keys_json`/`GlobalSettingsMsg`) — TF `/bind` state lives only in the
  server's own `TfEngine`, so a remote client can't run one locally.
- **The engine records, the App applies.** A TF command's output, sends, Clay commands,
  world definitions, connects, shell escapes and the rest are ordered `TfEffect`s
  (`src/tf/effects.rs`) collected per command frame; the App applies them through
  `src/tfrun.rs` (`run_tf_effects_console`/`_client`/`_daemon`, and
  `apply_tf_effects_now` + `deferred_tf` for synchronous callers such as hooks and
  triggers). New App code should run TF through those, not act on a
  `tf_engine.execute` result itself, or it drops everything but the result. A special
  variable bound to a Clay setting (`special_vars::BOUND`: more, wrapspace, isize,
  insert) emits `TfEffect::Setting(name, value)` - the value must travel with
  the effect, because `sync_tf_world_info` re-mirrors Clay's settings into those
  variables and would otherwise overwrite the new value before the effect is applied
  (an rc's `/addworld` before a later `/set` of one did exactly that). Clay has no
  non-visual mode (user ruling): `%visual` always reads `on`, and `/visual off` is
  refused (`special_vars::NO_NONVISUAL_MODE`).
- **TinyFugue startup** (`src/tfstartup.rs`): TF's own command line (`clay mymud`,
  `-f`, `-c`, `-L`, `-n`/`-l`/`-q`) and the first of `~/.tfrc`, `~/tfrc`, `./.tfrc`,
  `./tfrc`, run once at a cold start - never on a reload or crash restart, which carry
  the whole TF state over instead (`src/tf/snapshot.rs`, `tf_state=` in the reload
  file). The user ruled `-v` stays "version" and nothing auto-connects to the first world.
- **Tests never touch the real `~/.clay`**: under `cfg(test)` `get_home_dir()` is a
  per-process temp home and the engine's `HOME` points there too. A test that needs
  a file writes it under that home or `std::env::temp_dir()`. A test that needs a port
  for a server to bind takes `testserver::free_port()` - never "bind port 0, read it,
  drop it, bind again": the kernel can hand that same ephemeral port to another test in
  between, which is how "Address in use" failures came and went at random.
- **TF's status line** (`src/tf/status.rs`) replaces Clay's separator bar - in the
  console, web/GUI/Android (`#tf-status`, `layoutTfStatusRow` in app.js, a port of
  `status::layout_row`) and the SSH console - only once a script changes it
  (`StatusLayout::customized`); `App::refresh_tf_status` evaluates every world's view
  and sends `WsMessage::TfStatus` on change. Keep the Rust and JS layouts identical.
  **Separator Style is the look of TF's line too** (user ruling 2026-10-28): each
  interface pads with `_` on its output background for TinyFugue and with spaces on the
  status-bar colour for Web (`status::pad_for`; console `render_tf_status`, app.js
  `renderTfStatus` + `tf-status-tinyfugue`, native `chrome.rs status_bar`). `StatusView.pad`
  is non-empty only when a script set `%status_pad` (`StatusLayout::set_vars`, kept by
  `on_assign`/`on_unset`); a script's `%status_attr` applies in either look. The setting
  is applied through one path, `App::set_separator_style` (Settings in every interface,
  `/setup`, the Theme Editor's `UpdateSeparatorStyle`), which on TinyFugue → Web calls
  `status::reset_to_clay` (TF's layout and the `status_*` variables a script set back to a
  fresh engine's, the flag off) - the one way back to Clay's bar short of a cold start,
  since nothing else clears the flag and the TF snapshot carries it through `/reload`.
- **Two traps found the hard way**: `%;` (TF's command separator) is **not**
  quote-aware — a naive quote-aware splitter looks more correct but is wrong; real TF
  splits on every `%;` regardless of surrounding quotes. And TinyFugue does **not**
  expand `%var`/`$[...]`/`$(...)` on a bare top-level line read from a file — only
  inside a macro body, or via `/eval`'s own substitution pass; a script-test probe
  that needs expansion must be wrapped in a macro or `/eval`, and any engine change
  that substitutes a top-level file line unconditionally is reintroducing this bug.
