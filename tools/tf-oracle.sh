#!/usr/bin/env bash
# tf-oracle.sh - run tests/tf/cases/*.tf fixtures through real TinyFugue and
# print (or write) its filtered output, so Clay's own output can be graded
# against the real thing. See tests/tf/README.md ("Oracle" section) for the
# filtering rules this implements and how the Rust test
# `tf_script_oracle_diff` (src/tf/script_tests.rs) uses this script.
#
# Usage:
#   tools/tf-oracle.sh [--write] [case.tf ...]
#
# With no case arguments, every tests/tf/cases/*.tf is processed (sorted).
# Each case argument may be a path (absolute or relative to the current
# directory) or a bare case name (with or without ".tf"), resolved against
# tests/tf/cases.
#
# Without --write: prints, per case, a header line "== <name>" followed by
# TinyFugue's filtered output.
#
# With --write: writes that output to "<case>.expected" (no header) next to
# the .tf file, and prints one line per file written.
set -euo pipefail

if ! command -v tf >/dev/null 2>&1; then
  echo "tf-oracle.sh: 'tf' not found on PATH - install real TinyFugue (e.g. the tf5 package) to regenerate oracle output" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
CASES_DIR="$REPO_ROOT/tests/tf/cases"

WRITE=0
declare -a CASE_ARGS=()
for arg in "$@"; do
  if [[ "$arg" == "--write" ]]; then
    WRITE=1
  else
    CASE_ARGS+=("$arg")
  fi
done

declare -a CASE_FILES=()
if [[ ${#CASE_ARGS[@]} -eq 0 ]]; then
  while IFS= read -r -d '' f; do
    CASE_FILES+=("$f")
  done < <(find "$CASES_DIR" -maxdepth 1 -name '*.tf' -print0 | sort -z)
else
  for arg in "${CASE_ARGS[@]}"; do
    if [[ -f "$arg" ]]; then
      CASE_FILES+=("$(cd "$(dirname "$arg")" && pwd)/$(basename "$arg")")
    elif [[ -f "$CASES_DIR/$arg" ]]; then
      CASE_FILES+=("$CASES_DIR/$arg")
    elif [[ -f "$CASES_DIR/$arg.tf" ]]; then
      CASE_FILES+=("$CASES_DIR/$arg.tf")
    else
      echo "tf-oracle.sh: case not found: $arg" >&2
      exit 2
    fi
  done
fi

# Filtering, applied (in order) to TinyFugue's raw stdout+stderr. CSI
# sequences and bare keypad-toggle escapes are stripped INSIDE the awk
# script below (not by a separate sed pass beforehand, as this used to
# work) - see the rejoin step immediately below for why the stripping has
# to happen after the line-boundary decision, not before it:
#   - rejoin a terminal-width word-wrap: tf's own batch-mode output wraps
#     any physical line that would overflow the (real-terminal-less) column
#     width, always at a word boundary (leaving whatever trailing space
#     the original text already had at the break, consuming nothing of its
#     own) and always with a four-space hanging indent on the continuation
#     - this is presentation, not something the script itself printed, so
#     recover the single logical line by appending the continuation
#     (stripped of its four leading spaces) directly, with no space added
#     back (verified byte-for-byte against self.tf's own oracle output:
#     adding one produced a double space tf never printed). Applies
#     generally, not just to the two known-noise lines below (self.tf's own
#     quine output is long enough to wrap and must be read back as tf's own
#     single /echo line, not two) - repeated for a line wrapped more than
#     once. A blank line is never extended (nothing to overflow).
#
#     Telling a genuine wrapped continuation apart from a fresh new line
#     that merely happens to start with leading whitespace needs BOTH of
#     two signals, not just the leading-4-spaces check an earlier version
#     of this script used on its own: (1) every line tf itself draws is
#     preceded by "ESC [ K" (erase-to-end-of-line, from its internal
#     per-row redraw), while a wrapped continuation - being a terminal-side
#     artifact of where a row happened to overflow, not something tf drew
#     as its own row - carries no such marker (verified directly against
#     raw, un-stripped tf 5.0 beta 8 output: the copyright notice's wrapped
#     second line and self.tf's own wrapped quine line are the only two
#     raw lines with no "ESC [ K" anywhere in them); but (2) that alone
#     isn't sufficient either, because tf's own final on-exit echo of its
#     startup banner (every case's raw output ends with the banner text
#     repeated, verbatim, with NO redraw markers at all - apparently
#     printed directly rather than through the normal per-row redraw) also
#     lacks the marker, and would otherwise be wrongly merged into
#     whatever real content preceded it. The leading-4-spaces check alone
#     is equally insufficient on its own: it silently mismerges any
#     genuine tf output line that happens to start with 4+ spaces
#     (testcolor.tf's own column-ruler header,
#     "               01234567 01234567", into the immediately preceding,
#     soon-to-be-dropped "% Loading commands from ...testcolor.tf." banner
#     line - which is how that header went missing from
#     lib_testcolor.expected even though real tf does print it - the
#     ruler's raw line DOES carry a fresh "ESC [ K" marker, so signal (1)
#     alone correctly separates it). Requiring BOTH - no marker AND a
#     leading four-space indent once CSI-stripped - correctly classifies
#     all of: the two known real wraps (merge), the ruler line (marker
#     present - new line), and the repeated exit banner (no leading
#     4-space indent - new line, then dropped by the banner filter below
#     same as its first appearance). The rejoin decision below therefore
#     runs on the RAW line (marker still intact) and only strips CSI/
#     keypad-toggle escapes via strip_csi() once a line's fate (new vs.
#     continuation) is already settled.
#   - drop banner lines (startup/copyright/help-hint text, keyed by
#     substring - the copyright notice's own wrapped continuation, already
#     rejoined above, is caught this way too via "Ken Keys")
#   - drop only the specific "% ..." status lines that are tf's own startup
#     noise: "% Loading commands from ..." (once per /load or /require,
#     including tf's own stdlib.tf) and "% LC_..." (locale-category
#     messages) - already rejoined with any wrapped continuation above, so
#     dropping them here only ever removes one logical line. Every OTHER
#     "% " line is real script output (e.g. a library macro's own
#     `/echo -e %% ...` usage/error message, which comes out as a literal
#     "% " line) and must be kept.
#   - drop lines that are only "=" characters (keypad-mode escape residue)
#   - drop a final ">" prompt line, if present
#   - trim leading/trailing blank lines (terminal-init/redraw padding around
#     the banner - not part of the script's own output). Interior blank
#     lines are left alone in case a case ever legitimately echoes one.
read -r -d '' FILTER_AWK <<'AWK_EOF' || true
BEGIN { n = 0 }
{ n++; raw[n] = $0 }
function strip_csi(s) {
  gsub(/\033\[[0-9;?]*[A-Za-z]/, "", s)
  gsub(/\033[=>]/, "", s)
  return s
}
END {
  m = 0
  for (i = 1; i <= n; i++) {
    has_marker = (raw[i] ~ /\033\[K/)
    content = strip_csi(raw[i])
    is_continuation = (!has_marker && content ~ /^    /)
    if (m > 0 && logical[m] != "" && is_continuation) {
      cont = content
      sub(/^    /, "", cont)
      logical[m] = logical[m] cont
    } else {
      m++
      logical[m] = content
    }
  }

  k = 0
  for (i = 1; i <= m; i++) {
    line = logical[i]
    is_banner = (line ~ /TinyFugue version/ || line ~ /Copyright/ || line ~ /Ken Keys/ || line ~ /Type `/ || line ~ /PCRE/)
    is_drop_pct = (line ~ /^% Loading commands from/ || line ~ /^% LC_/)
    is_eq = (line ~ /^=+$/)
    if (!(is_banner || is_drop_pct || is_eq)) {
      k++
      arr[k] = line
    }
  }

  start = 1
  while (start <= k && arr[start] == "") start++
  last = k
  while (last >= start && arr[last] == "") last--
  if (last >= start && arr[last] == ">") last--
  while (last >= start && arr[last] == "") last--
  for (i = start; i <= last; i++) print arr[i]
}
AWK_EOF

run_case() {
  local abs_case="$1"
  local tmphome raw
  tmphome="$(mktemp -d)"
  raw="$(HOME="$tmphome" timeout 20 tf -n -v -q -f"$abs_case" </dev/null 2>&1 || true)"
  rm -rf "$tmphome"
  printf '%s\n' "$raw" | awk "$FILTER_AWK"
}

for abs_case in "${CASE_FILES[@]}"; do
  name="$(basename "$abs_case" .tf)"
  filtered="$(run_case "$abs_case")"
  if [[ "$WRITE" -eq 1 ]]; then
    expected_path="$(dirname "$abs_case")/$name.expected"
    printf '%s\n' "$filtered" > "$expected_path"
    echo "Wrote $expected_path"
  else
    echo "== $name"
    printf '%s\n' "$filtered"
  fi
done
