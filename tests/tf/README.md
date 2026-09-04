# TF-script test fixtures

This directory holds the Phase 0 test-script harness fixtures for Clay's
TinyFugue (TF) compatibility layer (`src/tf/`). Background and rationale are
in the plan at `investigate-differences-between-tinyfugu-fluffy-stallman.md`
("Findings" section, especially C.12) - read that first if something here is
surprising.

The runner itself is `src/tf/script_tests.rs` (`#[cfg(test)]`, declared from
`src/tf/mod.rs`). Run it with:

```
cargo test tf_script
```

or, for a single case:

```
TF_SCRIPT_CASE=strings cargo test tf_script
```

## How a case runs

For each `tests/tf/cases/<name>.tf`, the runner creates a fresh `TfEngine`,
loads the whole file through the same internal function `/load` and
`/require` use (`builtins::load_file_internal`), and records everything the
file produced into a `Transcript`: text that would have been shown to the
user (`echoed`), errors, text that would have been sent to a MUD
(`sent`), and text routed to Clay's own non-TF command dispatcher
(`clay_cmds` - `/quit` at the end of every case lands here, since a headless
run has no App to hand it to).

A case **passes** iff its `echoed` output exactly matches
`tests/tf/cases/<name>.expected` (trailing whitespace trimmed per line,
trailing blank lines ignored) **and** it produced no errors. The runner also
asserts the engine isn't left stuck in an unterminated `/if`/`/while`/`/for`
after the file finishes loading - see finding C.3.

## Oracle

Every `.expected` file is real TinyFugue's own output for the same `.tf`
file - Clay is being graded against the real thing, not a hand-written guess.
`tools/tf-oracle.sh` (plan step P0.4) automates producing that output with
real `tf` (installed at `/usr/bin/tf` on this machine, library at
`/usr/share/tf5/tf-lib`):

```
tools/tf-oracle.sh                  # print every case's filtered tf output
tools/tf-oracle.sh strings          # just one case (name, with or without .tf)
tools/tf-oracle.sh --write          # regenerate every tests/tf/cases/*.expected
tools/tf-oracle.sh --write foo.tf   # regenerate just one
```

Without `--write` it prints a `== <name>` header before each case's output
(so `--write` output and print-to-stdout output share one code path but
`.expected` files stay header-free). It exits 2 with a message if `tf` isn't
on `PATH`.

Each case runs as:

```
HOME=$(mktemp -d) timeout 20 tf -n -v -q -f/abs/path/to/case.tf </dev/null 2>&1
```

and the raw output is filtered down to exactly the script's own output:

- strip CSI sequences (`ESC [ ... final-byte`) and the bare `ESC =` / `ESC >`
  keypad-mode toggles.
- drop banner lines: startup/copyright/help-hint text, matched by substring
  (`TinyFugue version`, `Copyright`, `Ken Keys`, `` Type ` ``, `PCRE`) - this
  also catches the copyright notice's own line-wrapped continuation (via
  `Ken Keys`).
- drop only the specific `% ` lines that are tf's own startup noise: lines
  starting with `% Loading commands from` (once per `/load` or `/require`,
  including tf's own `stdlib.tf`) and lines starting with `% LC_` (locale-
  category messages, e.g. `` % LC_CTYPE category set to "en_US.UTF-8"
  locale. ``) - plus any line that wraps one of those two (four-space-
  indented, immediately following a dropped line; there can be more than
  one). Every *other* `% ` line is real script output and is kept - TF
  library macros routinely print their own usage/error text via `/echo -e
  %% ...`, which comes out as a literal `% ` line (e.g. map.tf's `/path`
  macro prints `% Path: <dir>`). Dropping every `% ` line indiscriminately,
  as an earlier version of this filter did, silently swallowed that
  legitimate output too.
- drop lines that are only `=` characters (keypad-mode escape residue the
  CSI strip doesn't fully swallow).
- drop a trailing `>` prompt line, if present (in practice `tf -n -v -q`
  ending on `/quit` never emits a bare one, but real tf can).
- trim leading and trailing blank lines. `tf`'s own startup/redraw sequence
  is bracketed by blank lines that carry no script output (terminal-init
  padding before the banner, and a spacer line before its own trailing
  banner echo on exit) - trimming only the outer run, not interior blank
  lines, is what makes `--write` reproduce the four checked-in `.expected`
  files byte-for-byte (verified with `git diff --stat tests/tf/cases`) while
  still leaving room for a future case that legitimately `/echo`s a blank
  line in the middle of its output.

`src/tf/script_tests.rs`'s `tf_script_oracle_diff` test runs every
non-xfailed case through this script live and diffs it against Clay's own
`run_script(...).echoed`, instead of the checked-in `.expected` snapshot
(that's what `tf_script_cases` does). It auto-skips (prints `SKIP: tf not on
PATH`, doesn't fail) when `tf` isn't installed, since a machine without real
TinyFugue has nothing to grade Clay against.

## Directives

A case file may start with `;;`-comment directive lines, read before the
first real command:

- `;; requires-lib` - this case needs the real TinyFugue library directory
  (`tf-lib`), which never ships in this repo (licensing). The runner resolves
  it as `$TFLIBDIR` if that's set and is a real directory, else
  `/usr/share/tf5/tf-lib` if that exists, else the case is **skipped** (not
  failed) with a printed reason. When a library dir is found, the runner sets
  the TF engine's own `TFLIBDIR` global variable to it before running, so a
  script that does `/require %TFLIBDIR/foo.tf` resolves the same way it would
  in a real, fully-configured `tf`.

- `;; preload: <file>` - before running the case, the runner loads
  `<tf_lib_dir>/<file>` (a bare filename, resolved against the same library
  directory `requires-lib` uses) through the same internal function
  `/load`/`/require` use, and discards that load's own result - except a
  load *error*, which is recorded into the transcript's `errors`, prefixed
  `"preload: "`, so a broken preload still shows up as a case failure rather
  than silently leaving the preloaded macros undefined. May be repeated to
  preload more than one file, in order. It implies `;; requires-lib` (the
  case is skipped, not failed, when no library directory is found).

  This directive is a **Clay-test-harness-only convenience** - real `tf`
  treats the `;;`-comment line as exactly that, a comment, and does nothing
  with it. That's fine: TinyFugue already has its whole stdlib loaded by the
  time any script file runs, so real `tf` never needs to `/require` it
  again. `;; preload:` exists so a Clay-only case can exercise stdlib or
  library macros (`/first`, `/nth`, `/escape`, ...) directly, without going
  through `/require`'s bare-filename search (finding C.2 - not implemented
  in Clay yet) just to load `stdlib.tf` or another pure library file.

## The xfail ledger (`xfail.txt`)

Some cases are *known* to fail today - they exist to pin down a specific,
already-diagnosed gap (see finding C in the plan) until a later Phase 1 step
fixes it. Each line is:

```
case-name | substring-of-expected-failure
```

Blank lines and `#`-comments are ignored. A case listed here **must** fail,
and its failure report (its errors, plus a description of the first
`echoed`/`expected` mismatch if there is one) must contain the given
substring - otherwise the runner reports the real failure text so the ledger
entry can be corrected. A listed case that unexpectedly **passes** is itself
reported as a failure ("remove it from xfail.txt"): that's the signal a
Phase 1 fix landed and the entry should move out, not stay as dead weight.

## Writing a new case: the C.12 rule

**TinyFugue does not expand `%var`, `$[...]`, or `$(...)` on a top-level line
read from a file.** That substitution only happens inside a macro body, or
when a command explicitly asks for it (`/eval` does one substitution pass on
its argument, then executes it). TinyFugue's own stdlib always wraps
expansions this way; it never relies on a bare top-level line being
expanded.

Clay, by contrast, currently substitutes every top-level command's arguments
unconditionally (see `execute_tf_command` in `src/tf/parser.rs`) - so a probe
written as a bare top-level `/echo len=$[strlen("abc")]` happens to work in
Clay today but would print the *literal*, unexpanded text under real `tf`.
That would make the two engines' output diverge for a reason that has
nothing to do with the thing actually being tested.

So: **every probe that needs expansion must be wrapped**, either by putting
it inside a macro body that the script then calls (as in `positional.tf`),
or by prefixing the top-level line with `/eval` (as in `strings.tf`):

```
/eval /echo len=$[strlen("abc")]
```

Both TinyFugue and Clay expand this the same way, so a mismatch in the
`.expected` comparison reflects a real behavioural difference, not an
artifact of how the probe was written.

## Other conventions

- Every case ends with `/quit`, so the exact same file also runs standalone
  under real `tf` (`tf -f case.tf`) for manual comparison. The runner just
  records it as a Clay-command pass-through, the same way `/quit` typed
  interactively would leave the TF engine and reach Clay's own dispatcher.
- Comment lines in a case body use `;` (TF's comment character), matching
  what `load_file_internal` itself recognises - see `;;`-prefixed directive
  lines above, which are a convention of this test harness only, not a
  TF syntax feature.

## Library cases

One `lib_<name>.tf` case per "pure" (no fake-MUD-server needed) file in the
real `tf-lib` (see the plan's finding D). Each starts with `;; requires-lib`
and `/require <name>.tf` (a bare filename, resolved by real `tf` via
`TFLIBDIR`). Originally every one of them failed at that very first line with
"Cannot find file: ..." (finding C.2); Job 6 implemented `/require`'s
bare-filename search, and each case's own gap is now whatever it probes
*after* that line loads successfully - see the "Current cases" table below
for each file's live PASS/XFAIL status (most now pass outright; the
remainder are tracked in `xfail.txt`, one small, already-diagnosed gap per
file - see the ledger's own comments for what job each is assigned to).

| Case | Library macros probed |
|---|---|
| `lib_lisp.tf` | `/car /cdr /cadr /length /reverse /remove /unique` |
| `lib_factoral.tf` | `rfact()`, `ifact()` |
| `lib_hanoi.tf` | `ismacro("hanoi")` only - `/hanoi`'s moves go to the MUD, not the screen |
| `lib_stack-q.tf` | `/push /pop /enqueue /dequeue` |
| `lib_tr.tf` | `/tr` |
| `lib_textencode.tf` | `/textencode /textdecode` |
| `lib_textutil.tf` | `%|` pipe operator via `/wc -w` and `/uniq` - see the file's own comment on why both use a macro rather than a bare command list |
| `lib_grep.tf` | `/fgrep` |
| `lib_alias.tf` | `/alias`, exercised via `/trigger -hSEND` rather than a live send - see the file's own comment |
| `lib_tools.tf` | `ismacro()` on `shl`, `name`, `xtitle`, `edmac` |
| `lib_kbstack.tf` | `ismacro()` on `kb_push`, `kb_pop` |
| `lib_kbregion.tf` | `ismacro()` on `kb_set_mark`, `kb_cut_region`, `kb_copy_region`, `kb_paste_buffer` |
| `lib_complete.tf` | `ismacro()` on `complete`, `complete_context`, `complete_variable` |
| `lib_quoter.tf` | `ismacro()` on `qdef`, `qfile`, `qtf`, `qsh` |
| `lib_cylon.tf` | `ismacro("cylon")` and `strlen(cylon0)` |
| `lib_map.tf` | `/mark /map /path /unmark` |
| `lib_spedwalk.tf` | `/speedwalk` toggled on then off (the file is really named `spedwalk.tf`, missing the first "e") |
| `lib_tick.tf` | `/tick`, `/ticksize` |
| `lib_at.tf` | `/at`'s usage message and `-v` future-time acceptance message |
| `lib_kbfunc.tf` | `ismacro()` on `dokey_home`, `dokey_end`, `kb_backward_kill_line` - see the file's own comment on why this is `ismacro()`-only rather than exercising cursor movement |
| `lib_kbbind.tf` | `ismacro()` on `key_up`, `dokey_home` (kbbind.tf transitively `/require`s kbfunc.tf) |
| `lib_worldq.tf` | `/list_active_worlds` |
| `lib_self.tf` | `/self` (a macro that prints its own body) |
| `lib_color.tf` | `strlen(start_color_red)`, `start_color_bgblue !~ ""` |
| `lib_testcolor.tf` | loading it directly (prints the colour tables as plain text once attributes are stripped) |
| `lib_tintin.tf` | `/showme`, `/math`, `/variable` - see the file's own comment on the one unavoidable `DEF: Redefined macro split` warning every load of this file produces |
| `stdlib_macros.tf` | `/first /rest /last /nth /escape /replace`, `isvar()`, `/toggle`, `/not`, `/expr` - via `;; preload: stdlib.tf`, not `/require` (see "Directives" above) |

## Current cases

| Case | Status | What it checks |
|---|---|---|
| `strings.tf` | PASS | Built-in string functions (`strlen`, `substr`, `strchr`, `strcat`, `toupper`, `tolower`, `pad`) via `/eval /echo`. |
| `positional.tf` | PASS | Positional-parameter forms in a macro body: `%{1-default}`, `%*`, `%{#}`, `%-1`, `%L`, `%-L` (finding C.5, fixed Job 8 - `/result`, `%{name-default}`, `%L %-N %-L` implemented in `variables.rs`). |
| `control_flow.tf` | PASS | A single-line `/if (1) cmd%; /endif`, with and without a space before the closing `%;` (finding C.3, fixed Job 6 - `%;/endif` with no whitespace is now recognised as closed). |
| `macro_result.tf` | PASS | `/result` used to make a macro behave as a callable function, both as `$[fn(...)]` and via `` $(/macro args) `` command substitution (finding C.5, fixed Job 8). |
| `eval.tf` | PASS | `/eval`'s own substitution pass: a plain variable, a variable holding a command *tail* run behind a literal `/`, and nested `$( $( ) )` command substitution. |
| `trigger.tf` | PASS | `/trigger` against real glob/regexp trigger patterns, including one that matches nothing (finding B, fixed Job 13 - `/trigger` now runs text through the real matcher, `macros::process_triggers`/`match_trigger`, instead of a pattern-string substring check). |
| `undefn.tf` | PASS | `/undefn %?` after `/def` (TF: `%?` holds the new macro's number) (finding B, fixed Job 13 - `/undefn` now removes by macro number, and `/def` sets `%?` to the new macro's number). |
| `macros.tf` | PASS | `/def -i`, `-q`, `-T<type>`, a nameless trigger macro, and a `-1` one-shot trigger (findings C.1, C.9, fixed Job 5 - `/def` flags `-i -q -I -f -T` and nameless macros). |
| `purge_args.tf` | PASS | `/purge <name>` and `/purge -mglob <pattern>` remove only matching macros (finding C.4, fixed Job 7 - `/purge`/`/list` now share a `MacroFilter` parsed from the same macro-option grammar; `/purge` is silent on success, matching real TF). |
| `for_syntax.tf` | PASS | TF's own `/for var min max command` form (finding C.7, fixed Job 9 - TF's form is now recognised when the 4th token isn't an integer step). |
| `if_command.tf` | PASS | `/if /command%; /then ... /else ... /endif` (finding C.8, fixed Job 9 - the command-form condition is now supported alongside the parenthesized form). |
| `at_prefix.tf` | PASS | The `/@name` builtin-bypass prefix, tested via `/@purge <name>` rather than shadowing a builtin with `/def` (finding C.6, fixed Job 8; see the file's own comment for why the shadow-and-bypass idiom from TF's docs doesn't produce a portable fixture here). |
| `hooks.tf` | PASS | Hook events beyond Clay's original 11 (all 31 now parse), and the combined `-h"EVENT pattern"` syntax, exercised via `/trigger -h<event>` rather than a live send (finding C.10, fixed Job 10; see the file's own comment for why a plain unconnected line can't be used). |
| `functions.tf` | PASS | Functions from finding C.11 plus `replace()`'s TF argument order (a B ruling) - fixed Job 11. |
| `time.tf` | PASS | `/time` itself is clock-dependent and untestable; this only adds a second, different `ftime()` format string beyond the ones in `functions.tf`. |
| `lib_alias.tf` | PASS | `/alias`, exercised via `/trigger -hSEND` rather than a live send - see the file's own comment |
| `lib_at.tf` | PASS | `/at`'s usage message and `-v` future-time acceptance message (fixed Job 15b-i - see xfail.txt's own removed entry for the chain of fixes: aggregate_results_with_engine losing prior echo'd text ahead of a /return, `%0`, and `%{P1-$[...]}` defaults). |
| `lib_color.tf` | PASS | `strlen(start_color_red)`, `start_color_bgblue !~ ""` (fixed Job 15b-i - TF's real "%"/"$" escaping rule is a run of N >= 2 collapsing to N - 1 literal characters, not a pairwise halving; see xfail.txt's own removed entry). |
| `lib_complete.tf` | PASS | `ismacro()` on `complete`, `complete_context`, `complete_variable` |
| `lib_cylon.tf` | PASS | `ismacro("cylon")` and `strlen(cylon0)` |
| `lib_factoral.tf` | PASS | `rfact()`, `ifact()` |
| `lib_grep.tf` | PASS | `/fgrep` |
| `lib_hanoi.tf` | PASS | `ismacro("hanoi")` only - `/hanoi`'s moves go to the MUD, not the screen |
| `lib_kbbind.tf` | PASS | `ismacro()` on `key_up`, `dokey_home` (kbbind.tf transitively `/require`s kbfunc.tf) |
| `lib_kbfunc.tf` | PASS | `ismacro()` on `dokey_home`, `dokey_end`, `kb_backward_kill_line` - see the file's own comment on why this is `ismacro()`-only rather than exercising cursor movement |
| `lib_kbregion.tf` | PASS | `ismacro()` on `kb_set_mark`, `kb_cut_region`, `kb_copy_region`, `kb_paste_buffer` |
| `lib_kbstack.tf` | PASS | `ismacro()` on `kb_push`, `kb_pop` |
| `lib_lisp.tf` | PASS | `/car /cdr /cadr /length /reverse /remove /unique` (fixed Job 15b-ii - `$(...)`/`$[...]` inside an expression are now real operands, resolved lazily by the evaluator, plus `parser::parse_macro_args`'s own bogus "delimited pattern" heuristic - which coincidentally mismerged an ordinary word list whenever it started and ended with the same letter - was removed entirely; see xfail.txt's own removed entry). |
| `lib_map.tf` | PASS | `/mark /map /path /unmark` |
| `lib_quoter.tf` | PASS | `ismacro()` on `qdef`, `qfile`, `qtf`, `qsh` |
| `lib_self.tf` | PASS | `/self` (a macro that prints its own body) (fixed Job 15b-i - `control_flow::split_percent_semi`'s "%;" splitting was quote-aware, but real tf's own body-splitting is NOT; see xfail.txt's own removed entry). |
| `lib_spedwalk.tf` | PASS | `/speedwalk` toggled on then off (the file is really named `spedwalk.tf`, missing the first "e") (fixed Job 15b-i - `/ismacro` now forces `-i` the way real tf's own stdlib macro does, so it can see spedwalk.tf's own invisible hook macro; see xfail.txt's own removed entry). |
| `lib_stack-q.tf` | PASS | `/push /pop /enqueue /dequeue` (fixed Job 15b-ii - same `$(...)`/`$[...]`-as-expression-operand fix as lib_lisp, plus a real-tf-verified unbraced "%N-default" form - `%1-queue` - in `variables::substitute_variables`'s digit-selector arm; see xfail.txt's own removed entry). |
| `lib_testcolor.tf` | XFAIL | loading it directly (prints the colour tables as plain text once attributes are stripped) - progressed a great deal further in Job 15b-ii (oracle \r/redraw-marker fix restored the missing ruler header, `normalize_echoed_lines` fixed an ANSI-comparison harness gap, LOADFAIL + `;; preload: stdlib.tf` fixed the missing `_echo`, and a real getopts() bug - a bare "-" end-of-options marker wasn't being consumed - was fixed); remaining gap (a positional-parameter whitespace-fidelity architecture question, not "one small bug") tracked in xfail.txt. |
| `lib_textencode.tf` | PASS | `/textencode /textdecode` (fixed Job 15b-ii - `regmatch()` now also updates the P0-P9/PL/PR LOCAL VARIABLES a trigger match sets, not just the separate array the "%P0" TEXT-substitution form reads - the bare `{P0}`/`{PL}`/`{PR}` EXPRESSION-brace form only ever checked locals; see xfail.txt's own removed entry). |
| `lib_textutil.tf` | XFAIL | `%|` pipe operator via `/wc -w` and `/uniq` - see the file's own comment on why both use a macro rather than a bare command list - remaining gap tracked in xfail.txt (job 16 (%| pipe operator)). |
| `lib_tick.tf` | PASS | `/tick`, `/ticksize` |
| `lib_tintin.tf` | PASS | `/showme`, `/math`, `/variable` - see the file's own comment on the one unavoidable `DEF: Redefined macro split` warning every load of this file produces (fixed Job 15b-ii - finding 34's LOADFAIL fix let `;; preload: stdlib.tf` finally complete cleanly, making tintin.tf's own "split" redefinition genuine; see xfail.txt's own removed entry). |
| `lib_tools.tf` | PASS | `ismacro()` on `shl`, `name`, `xtitle`, `edmac` |
| `lib_tr.tf` | PASS | `/tr` |
| `lib_worldq.tf` | PASS | `/list_active_worlds` |
| `stdlib_macros.tf` | PASS | stdlib.tf one-liners (`/first /rest /last /nth /escape /replace /toggle /not /expr`, `isvar()`) via `;; preload: stdlib.tf` rather than `/require` (fixed Job 15b-ii - finding 34: `load_file_internal` was already firing the LOADFAIL hook but discarding its `HookOutcome`, so stdlib.tf's own gagged guard around the optional, legitimately-missing `local.tf` could never suppress the error; preloading stdlib.tf now completes with zero errors end to end; see xfail.txt's own removed entry). |

**`dokey.tf` was planned but is not included.** TinyFugue's own non-visual/quiet batch mode (`tf -n -v -q -f...`) still emits raw terminal control bytes - literal `\r` carriage-returns, runs of `\x08` backspaces, and `\a` bells - to keep an internal command-line "redraw" in sync every time `/input` or a buffer-changing `/dokey`/`/dokey_*` runs, even with no real terminal attached. Those bytes are stable and reproducible run-to-run, but they're pure terminal-redraw noise that Clay's headless, engine-only test harness (no `App`, no crossterm rendering) structurally can never emit itself, even after `/dokey` and `kb*()` state syncing are fully implemented (Phase 1/2) - so a fixture built on them could never move from XFAIL to PASS, which defeats the point of the ledger. `kbpoint()`/`kblen()`/`kbhead()`/`kbtail()` on a buffer nothing has ever touched are already covered as "Already working" in the plan's finding C and by existing engine tests, so a fixture limited to that baseline wouldn't add anything either. `lib_kbfunc.tf` (which probes the same library `dokey_home`/`dokey_end`/`kb_backward_kill_line` macros) hits the identical problem and was adapted the same way - see its own file comment.

Note: `strings.tf` deliberately does not probe `replace()` - Clay's
`replace(str, old, new)` takes its arguments in a different order than real
TinyFugue's `replace(old, new, str)` (TF: "returns `str` with every
occurrence of `old` replaced by `new`"). This wasn't one of the gaps listed
in the plan's finding C; it surfaced while writing this fixture. It isn't
tracked as an xfail case yet because that needs its own decision (which
order is "right" for Clay to standardize on) rather than a Phase 1 fix to an
already-agreed ruling - flag it for whoever picks up Phase 1's `replace()` /
Bucket-A work.
