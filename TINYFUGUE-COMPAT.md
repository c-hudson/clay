# Clay TinyFugue Compatibility — Design Record

**Status: implemented, 2026-09.** This document is the authoritative record of the
rulings behind Clay's TinyFugue (TF) 5.0 compatibility layer (`src/tf/`), its keybinding
grammar and dispatch order (`src/keynames.rs`, `src/chords.rs`, `src/keybindings.rs`),
and the TF-script test suite (`tests/tf/`, `tools/tf-oracle.sh`) — the result of an
investigation that compared Clay against real TF 5.0 beta 8 (installed at `/usr/bin/tf`,
library `/usr/share/tf5/tf-lib`) both by reading source and by driving both engines
side by side. User-facing summaries live in `docs/markdown/06-tf-commands.md` (command
set, differences) and `docs/markdown/07-keyboard-shortcuts.md` (keys, generated from
code and kept honest by `cargo test test_docs_key_table_matches_defaults`).

Scope decided up front and not re-litigated:

- **Fixtures reference the *system* `tf-lib`** (`$TFLIBDIR`, else
  `/usr/share/tf5/tf-lib`); library-dependent tests skip when it's absent. **Nothing
  GPL-licensed enters the Clay repo** — the real TF library is only ever referenced on
  a machine that already has it installed (`apt install tf5`).
- **Keys**: TinyFugue's own defaults become Clay's defaults, **except `^Q` (spell
  suggestions) and `^R` (hot reload) stay Clay's**. Every default, chords included,
  remains editable in `keybindings.dat` and the web keybind editor.
- **Commands**: every Clay-only option is kept (e.g. `/recall -D`); TF's missing
  options were added; where the same command means two different things, **TF wins**.

## A. Key Bindings

TF binds raw byte sequences and chords (`/bind ^X^R`, `/def -b'^[[A'`), has a numeric
prefix `%kbnum` (`Esc-0..9`, `Esc--`), an insert/overwrite toggle, a `key_<name>` macro
layer, and 35 `/dokey` primitives. Clay's pre-parity model bound one crossterm event
name to an action id with no chord/kbnum/insert support at all. The rewrite:

- **One canonical key-name grammar** (`src/keynames.rs`), shared by `keybindings.dat`,
  `/bind`, `key_event_to_name`, `app.js`, and the keybind editor. TF's raw spellings
  (`^[b`, `\033`, `\0x1B`, `\27`, raw terminal escape sequences like `^[[1;5A`) are
  accepted and normalised into it. Lookups are case-preserving (`Esc-j` != `Esc-J` —
  the pre-parity code upper-cased everything and collided the two).
- **Chords** (`src/chords.rs`, `ChordState`): any keystroke that could be the first
  half of a longer binding buffers for `DEFAULT_CHORD_WINDOW` = 500ms; `^G` cancels
  immediately. Console (`input_handler::handle_key_event`) and the SSH remote console
  (`remote_client::handle_remote_client_key`) share the exact same
  `chords::resolve_key_name` — this used to be two separately-drifting
  implementations.
- **Four-level dispatch order** for every keypress, identical on console and
  web/GUI/SSH-console clients:
  1. A `/bind`/`/def -b`/`/def -B` match for the exact key (or chord) pressed.
  2. A `key_<name>` macro (TF's own two-level named-key naming — `key_f5`,
     `key_ctrl_left`, `key_esc_left`; `key_meta_<x>` falls back to `key_esc_<x>`).
  3. The built-in action table (`ACTIONS` + `dispatch_action`, `keybindings.dat`).
  4. Ordinary literal character input.
- **`kbnum`** (`InputArea.kbnum`) and **insert mode** (`InputArea.insert`), mirrored
  into TF globals `%kbnum`/`%insert` so `/dokey` and script logic see the same state a
  keypress does.

### Per-key rulings (where TF and Clay's pre-parity defaults differed)

| Key | TF | Clay pre-parity | Ruling |
|---|---|---|---|
| `^Q` | literal-next | spell suggestions | **Clay** |
| `^R` | refresh line | hot reload | **Clay** |
| `Ctrl-Up/Down` | recall history | switch active world | **TF**; world switching moves to `Esc-Left/Right` (TF socket cycling), `Esc-{`/`Esc-}` (Clay's unseen-first cycling), `Shift-Up/Down` |
| `Tab` | page | completion if input starts with `/`, else page | **Clay**, plus `Esc-Tab` = completion |
| `^L` | repaint | repaint + drop client-generated lines | **TF**; the filtering repaint becomes action `redraw_server_only`, unbound by default |
| `^U` | kill to start of line | clear whole line | **TF** (kill ring kept; whole-line clear = `clear_line`, TF's own `/dokey DLINE`, unbound by default) |
| `Esc--` / `Esc-=` | kbnum negative / goto bracket | goto bracket / unbound | **TF** |
| `^S`, `Insert`, `Esc-v`, `Ctrl-Left/Right`, `Ctrl-Home/End`, `Ctrl-PgDn`, `Esc-<`/`>`, `Esc-^N`/`^P`, `Esc-^L`, `Esc-^E`, `Esc-L`, `Esc-0..9`, `^X^R ^X^V ^X^? ^X[ ^X] ^X{ ^X}`, `^]` | bound | unbound | **added** (pure additions, no conflict) |
| `^Y`, `Shift-Up/Down`, `F2 F4 F5 F8 F9`, `Alt-Up/Down`, double-`^C`, `Esc-w` | unbound | bound | **kept** (Clay extras) |

Identical already (no change either direction): `^A ^B ^D ^E ^F ^G ^K ^N ^P ^T ^V ^W`,
`Esc-b/f/c/d/l/u`, `Esc-Space`, `Esc-.`/`_`, `Esc-p/n`, `Esc-j/J`, `Esc-h`,
`Esc-Backspace`, `Up/Down`, `Home/End`, `PgUp/PgDn`, `Delete`, `F1`.

**Real bugs found and fixed along the way**: `/bind Esc-j` used to be stored as
`Esc-j` but looked up as `Alt-J` so it never fired, and `Alt-j`/`Alt-J` collided under
one upper-cased form; nameless macros (`-b` with no name) were rejected outright, so
every `kbbind.tf`-style `-b` definition failed; `/dokey` accepted only 20 of TF's 35
names, mapped `HPAGE` to page-*up* instead of half-page-forward, and its
`UP DOWN NEWLINE FLUSH PAGE HPAGE SEARCHB SEARCHF PAUSE` names emitted internal
`__dokey_*` strings that nothing ever consumed (dead code since whenever they were
added).

## B. Commands

Rulings for same-name commands where TF and Clay disagreed on meaning:

| Command | TF | Clay pre-parity | Ruling |
|---|---|---|---|
| `/eval` | one more substitution pass, then execute (`-s<n>`) | pass-through, no substitution | **TF** |
| `/time` | `/time [<format>]` prints the time | `/time [/cmd]` times a command | **both**: format form per TF; `/time /cmd` keeps timing; added `/runtime` |
| `/trigger` | `[-ln] [-g] [-w<world>] [-h<event>] <text>` runs text through the real matcher | fired macros whose pattern overlapped the text as a substring; `-d` deleted | **TF**, kept `-d` |
| `/undefn` | by macro number | by name pattern | **TF** (pattern removal is now `/purge -mglob`) |
| `replace()` function | `replace(old, new, string)` | `replace(string, old, new)` | **TF** (Clay's `/replace` *command* already used TF's order — only the function changed; see "Changed defaults" below) |

TF options added while keeping every Clay-only option: `/recall -a<attrs>
-A/-B/-C<n> #`; `/send -W -T<type> -n -h`; `/echo -p -e -o -A -r`; `/log -l -i -g
-w<world> on`; `/world -q -n -x` + `<host> <port>`; `/fg -n -s -q -l -c<N> -< ->`;
`/listsockets -s -n -m -S -T`; `/listworlds -c -u -s -m -S -T`; `/addworld -s<srchost>
[file] DEFAULT`; `/list`/`/purge` macro-option filters; `/hook <event>[ <pattern>] [=
body]`, `/hook on|off`, `/unhook <event> [<pattern>]`; `/grab <text>` (was a stub);
multiple args for `/kill /undef /unworld`; `/shift [n]`, `/break [n]`, `/exit [n]`;
`/dc [<world>|-ALL]`; `/save -a` + list filters; `/histsize -w`; `/ps -s -r -q -w
[pid]`; `/listvar -m -g -x -s -v [name [value]]`; `/lcd` bare + `/cd` `/pwd`; `/sh -q`
+ bare `/sh`; `/beep on|off`; `/substitute -p`; fixed `/quote`'s own help text (it
described flags the implementation never had).

Missing TF builtins/stdlib one-liners implemented natively (no shipped stdlib, so they
survive hot reload with no three-UI plumbing): `/result /features /limit /unlimit
/relimit /then /do /restrict /core /complete /ismacro /first /rest /last /nth /isvar
/more /wrap /sys /runtime /ver /cd /pwd /man /nogag /true /false /:`. Optional-library
commands (alias, at, lisp, stack-q, textencode, textutil, map, speedwalk, tick, tools,
quoter, watch, spell, psh, color) come from the real library itself once `/require`
can find it — see "Loading TinyFugue's own library" below.

## C. Engine Gaps (all reproduced against real TF, all fixed except the two noted)

The investigation ran all 44 pure `tf-lib` scripts through Clay's engine, one per fresh
process, and found every one of them failed at the time. Headline gaps, now fixed:
`/def` rejected `-i -q -I -T -f -s`; `/require` had no `%TFLIBDIR`/`%TFPATH` search;
a single-line `/if …%;/endif` with no space before the closing `%;` locked the engine
up permanently (`ControlState::If` swallowed every later line); `/purge` ignored its
own arguments and deleted every macro; `/result` was missing entirely and
`%{1-default}`/`%L`/`%-N`/`%-L` were not substituted; `/@cmd` (builtin-bypass) was
unknown; TF's own `/for var min max command` form was misparsed as Clay's numeric-step
form; the command-form `/if /command%; /then …` was rejected outright; nameless macros
were rejected; only 11 of TF's 31 (now 32) hook events were recognized and
`-h"EVENT pattern"` was rejected; several functions were missing entirely
(`features mktime cputime ln morepaused winlines strip_attr encode_attr decode_attr
encode_ansi decode_ansi strcmpattr is_open gethostname status_fields spam`, function
forms of `prompt eval def test`); TinyFugue does **not** expand `%var`/`$[]`/`$()` on
a bare top-level line read from a file, but Clay used to unconditionally — see
"Loading TinyFugue's own library" below for the rule this became.

**Remaining known gaps** (tracked in `tests/tf/xfail.txt`, both small and isolated —
neither blocks ordinary use):

- `lib_testcolor` — `/echo`'s argument loses double-space fidelity once split into
  positional parameters; real TF's own `getopts()`/`shift()` preserve original
  argument-text spacing internally in a way Clay's macro-invocation plumbing doesn't
  yet reproduce.
- `lib_textutil` — TF's `%|` pipe operator (tfio streams) is unimplemented.

Other gaps fixed but worth knowing about: `/set`/`/let` used to trim their value,
losing meaningful leading/trailing whitespace; `:=` assignment always wrote the
innermost local scope instead of updating an existing binding wherever it lives;
`/def` didn't accept TF's bundled short options (`-iFp9999`); `/return`/`/result`
used to be dropped when used inside a nested `/if`/`/while`/`/for` block; `{-N}` meant
"Nth from the end" instead of TF's actual "all but the first N"; the comma operator
was missing; a loaded file discarded every successfully-echoed line the moment any
*later* line in the same file errored (real TF interleaves output and errors);
`/dokey` accepted 20 of 35 names; `/def` redefinition didn't print TF's `%
Redefined macro X` message and `/undef`/`/purge` weren't silent on success; a native
Clay stub (`/tick`, `/telnet`, …) always shadowed a same-named library macro instead of
the reverse (TF rule: a user macro shadows a builtin, `/@name` forces the builtin);
`%%;` (a literal, doubled `%`) was mishandled by the body splitter; a bogus "delimited
pattern" heuristic in argument parsing silently mismerged ordinary word lists that
happened to start and end with the same letter; `regmatch()` updated only the
text-substitution array, not the `{P0}`-style expression-brace locals a real trigger
match also sets; nested command-form `/for` loops could hang the engine; `LOADFAIL`
was parsed but never actually fired, so a gagged guard around an optional,
legitimately-missing file (stdlib.tf's own `local.tf` guard) could never suppress its
error.

## The Script-Test Suite

`tests/tf/` (harness: `src/tf/script_tests.rs`, `#[cfg(test)]`, declared from
`src/tf/mod.rs`). Run with `cargo test tf_script`, or `TF_SCRIPT_CASE=<name> cargo
test tf_script` for one case.

- **Fixtures**: `tests/tf/cases/<name>.tf` + `<name>.expected`. A case passes iff its
  echoed output exactly matches `.expected` (trailing whitespace trimmed per line,
  trailing blank lines ignored) and produces no errors; the runner also asserts the
  engine isn't left stuck in an unterminated `/if`/`/while`/`/for` after the file
  finishes. One `lib_<name>.tf` case exists per pure (no fake-MUD-needed) file in the
  real `tf-lib`; each begins with a `;; requires-lib` directive and is **skipped**
  (not failed) when no library directory can be found.
- **Oracle**: `tools/tf-oracle.sh [--write] [case...]` runs the real `tf` binary
  (`HOME=$(mktemp -d) timeout 20 tf -n -v -q -f<case> </dev/null`) and filters its raw
  terminal output down to exactly the script's own text — see the script's own
  comments for the filtering rules (CSI/keypad-toggle stripping, banner/loading-noise
  removal, terminal-width-wrap rejoining keyed off tf's own `ESC[K` per-line redraw
  marker). `#[test] tf_script_oracle_diff` runs this live and diffs Clay's own output
  against it for every non-xfailed case, auto-skipping when `tf` isn't on `PATH`.
- **`%TFLIBDIR` resolution** for the test harness: `$TFLIBDIR` env var if it names a
  real directory, else `/usr/share/tf5/tf-lib` if that exists, else the case is
  skipped. Identical to the engine's own runtime default (`src/tf/mod.rs`).
- **The xfail ledger** (`tests/tf/xfail.txt`): `case-name | substring-of-expected-failure`,
  one line per known-failing case. A listed case **must** fail with a report containing
  that substring, or the runner reports the real failure so the ledger entry can be
  corrected; a listed case that unexpectedly **passes** is itself a test failure ("remove
  it from xfail.txt") — the signal that a fix landed and the entry is stale.
- **The C.12 rule**: TinyFugue does not expand `%var`/`$[...]`/`$(...)` on a bare
  top-level line read from a file — only inside a macro body, or via `/eval`'s own
  substitution pass. Every fixture probe that needs expansion must be wrapped in a
  macro body or prefixed with `/eval`, or the two engines' output would diverge for a
  reason that has nothing to do with the thing actually being tested. See
  `tests/tf/README.md`'s "Writing a new case" section for the worked example.

## Intentional Differences (kept, not bugs)

See `docs/markdown/06-tf-commands.md`'s "Differences from TinyFugue" section for the
user-facing version. Summary: `^Q`/`^R` stay at their Clay meanings; `Tab` keeps
paging/more-mode priority (`Esc-Tab` does TF-style completion); the kill ring's `^Y`,
the F-keys, `Shift-Up/Down`, and `Alt-Up/Down` are Clay-only additions; `/recall -D`,
`/world -e`, `/watchdog -w<world>`, `/trigger -d`, `/repeat -p<priority>`, long-form
`/def -a"gag"`, `#`/`# ` comments in `/load`, `/quote -A -P`, and `/tfhelp` are
Clay-only extras kept alongside TF's own behavior; `/limit`/`/unlimit`/`/relimit` and
`/xtitle` are console-only (no remote-filter wire message exists yet); the
`expand_line` key action is a no-op on the plain web/GUI client (no safe wire path for
a server to substitute a remote client's own input line).

## Changed Defaults — Release Note

Anyone upgrading into this keymap will see these defaults change from Clay's
pre-parity behavior:

- **`Ctrl-Up`/`Ctrl-Down`** now recall command history instead of switching worlds.
- **World switching** moved off `Ctrl-Up`/`Ctrl-Down` onto three pairs:
  `Esc-Left`/`Esc-Right` cycle *connected* worlds (TF's own SOCKETB/SOCKETF),
  `Esc-{`/`Esc-}` run Clay's "unseen-first" cycling (`world_prev`/`world_next`,
  governed by the "World Switching" setting), and `Shift-Up`/`Shift-Down` cycle
  *all* worlds (unchanged). TF binds `Esc-{`/`Esc-}` to socket cycling as well;
  Clay gives that redundant pair to its own cycling so both styles keep a key.
- **`^L`** no longer drops client-generated lines on repaint — it's now a plain
  refresh (TF REFRESH). The old behavior is the separate `redraw_server_only` action,
  unbound by default.
- **`^U`** now kills to the start of the line (kill ring kept) instead of clearing the
  whole line. The old behavior is the separate `clear_line` action (TF's own `/dokey
  DLINE`), unbound by default.
- **`Esc--`** now starts a numeric prefix (`%kbnum`); **`Esc-=`** is now "goto matching
  bracket" (previously `Esc--` did this).
- **`replace()`** (the expression function, not the `/replace` command) now takes
  TF's own argument order, `replace(old, new, str)`, instead of Clay's previous
  `replace(str, old, new)`. Any script using the function form needs its arguments
  reordered.
- **`/bind`** now defers substitution to keypress time (matching `/def`'s own body
  semantics) instead of substituting once, eagerly, when the binding was typed.

**If you want the old keys back**, add these lines under `[bindings]` in
`~/.clay/keybindings.dat` (or set them the same way in the web keybind editor):

```ini
Ctrl-Up = world_next
Ctrl-Down = world_prev
^L = redraw_server_only
^U = clear_line
Esc-- = goto_matching_bracket
```

Any binding you had already customized survives this change untouched — `keybindings.dat`
only ever stores the keys you explicitly set or explicitly `UNBOUND`; everything else
tracks the compiled-in default table, which is exactly what changed here.

## Appendix: Every Default Binding

Mirror of `PINNED_DEFAULTS` in `src/keybindings.rs`, sorted by key. This copy is the one
`test_docs_key_table_matches_defaults` requires: `docs/` is gitignored (it holds the
generated-PDF sources), so the same table in `docs/markdown/07-keyboard-shortcuts.md` is
only checked on a machine that has it. Changing a default means changing all three.

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
| `^L` | `refresh_line` |
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
