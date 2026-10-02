//! Phase 0 TF-script test harness (see the "investigate differences between
//! tinyfugue fluffy stallman" plan). Runs a whole `.tf` fixture file through
//! the TF engine exactly the way `/load`/`/require` would, headlessly (no
//! App, no terminal), and records what it produced so it can be compared
//! against real TinyFugue's output for the same file.
//!
//! This module makes NO engine-behaviour changes itself; it is a harness only.
//! The one production change it depends on is `builtins::load_file_internal`
//! becoming `pub(crate)` and aggregating a loaded file's own output instead of
//! discarding it on success (see that function's doc comment) - both changes
//! live in `builtins.rs`, not here.
//!
//! See `tests/tf/README.md` for the fixture format, directives, and the xfail
//! ledger (`tests/tf/xfail.txt`).

use super::control_flow::ControlState;
use super::{TfCommandResult, TfEngine, TfValue};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Everything a fixture run produced, sorted into the same buckets a real
/// App/TUI would route a `TfCommandResult` to.
pub(crate) struct Transcript {
    /// Text that would have been displayed to the user (from `/echo`, etc.).
    pub(crate) echoed: Vec<String>,
    /// Error text (from a bad command, a failed load, or the harness's own
    /// end-of-script sanity check).
    pub(crate) errors: Vec<String>,
    /// Text that would have been sent to a MUD server.
    pub(crate) sent: Vec<String>,
    /// Text that would have been routed to Clay's own (non-TF) command
    /// dispatcher (e.g. `/quit`), plus short descriptions of anything else
    /// that needs an App to actually carry out (a `/recall`, a `/repeat`).
    pub(crate) clay_cmds: Vec<String>,
}

/// Run a whole `.tf` file through the TF engine and collect everything it did.
///
/// Loads `path` via `builtins::load_file_internal` (quiet, so the "Loading
/// commands from ..." message never pollutes `echoed`), then drains whatever
/// the load queued for a MUD or a keyboard op, and finally sanity-checks that
/// the engine did not get stuck in an unterminated `/if`/`/while`/`/for`.
pub(crate) fn run_script(engine: &mut TfEngine, path: &Path) -> Transcript {
    let mut transcript = Transcript {
        echoed: Vec::new(),
        errors: Vec::new(),
        sent: Vec::new(),
        clay_cmds: Vec::new(),
    };

    let path_str = path.to_string_lossy().to_string();
    let result = super::builtins::load_file_internal(engine, &path_str, true);

    // No effect frame is open here, so everything the load did is in the engine's
    // top-level queue, in the order it happened (see `super::effects`).
    for effect in engine.take_effects() {
        record_effect(&mut transcript, effect);
    }
    record_result(&mut transcript, result);

    // Don't let a script's keyboard manipulation (kbgoto, kbdel, ...) leak
    // into whatever case runs next - each case gets a fresh TfEngine, but
    // nothing else clears this, and it isn't part of what we're comparing.
    engine.pending_keyboard_ops.clear();

    if !matches!(engine.control_state, ControlState::None) {
        transcript.errors.push(
            "engine left in control-flow state (unterminated /if, /while or /for)".to_string(),
        );
    }

    transcript
}

/// True for a line that is only Clay's own file-load noise ("Loading
/// commands from <path>", emitted by `builtins::load_file_internal` for any
/// non-quiet load). The *top-level* case file is always loaded quiet
/// (`run_script` passes `quiet: true`) specifically to keep this line out of
/// `echoed`, but a bare `/require`/`/load` line *inside* that file (which is
/// how every real TF library file requires another one - see finding C.2)
/// is not quiet, and real TinyFugue prints the equivalent `% Loading
/// commands from ...` line for those too - `tools/tf-oracle.sh` filters
/// that same line out of real `tf`'s raw output as startup noise (see
/// `tests/tf/README.md`'s "Oracle" section). Filtering it here as well
/// keeps the comparison apples-to-apples instead of every nested
/// `/require` line permanently mismatching a `.expected` file that (via the
/// oracle) never contains it.
fn is_load_noise_line(line: &str) -> bool {
    line.starts_with("% Loading commands from ")
}

/// Sort one effect into the right `Transcript` bucket - the buckets a real App/TUI
/// routes each effect to (see `tfrun`), since this harness has no App to hand them to.
fn record_effect(transcript: &mut Transcript, effect: super::effects::TfEffect) {
    use super::effects::TfEffect;
    match effect {
        TfEffect::Output { text, .. } => {
            // Split as `App::emit_client_text` does: a final newline ends the text, it
            // doesn't add an empty line.
            transcript.echoed.extend(
                text.strip_suffix('\n').unwrap_or(&text).split('\n')
                    .map(|s| s.to_string())
                    .filter(|line| !is_load_noise_line(line)),
            );
        }
        TfEffect::Error { msg, at } => {
            let msg = match at { Some(at) => format!("{}: {}", at, msg), None => msg };
            transcript.errors.extend(msg.split('\n').map(|s| s.to_string()))
        }
        TfEffect::Send { text, .. } => transcript.sent.push(text),
        TfEffect::Clay { cmd, .. } => transcript.clay_cmds.push(cmd),
        TfEffect::Quote(q) => match q.disposition {
            super::QuoteDisposition::Echo => transcript.echoed.extend(q.lines),
            super::QuoteDisposition::Send => transcript.sent.extend(q.lines),
            super::QuoteDisposition::Exec => transcript.clay_cmds.extend(q.lines),
        },
        TfEffect::Unknown(cmd) => transcript.errors.push(format!("Unknown command: {}", cmd)),
        TfEffect::Recall(_) => transcript.clay_cmds.push("<recall>".to_string()),
        TfEffect::StartProcess(process) => {
            transcript.clay_cmds.push(format!("<repeat process {}>", process.id));
        }
        TfEffect::WorldOp(op) => transcript.clay_cmds.push(format!("<addworld {}>", op.name)),
        TfEffect::Connect(req) => transcript.clay_cmds.push(format!("<connect {:?}>", req)),
        TfEffect::Setting(..) => {}
        TfEffect::Shell { command } => transcript.clay_cmds.push(format!("<sh {}>", command.unwrap_or_default())),
        TfEffect::Suspend => transcript.clay_cmds.push("<suspend>".to_string()),
        TfEffect::SetPrompt { text, .. } => transcript.clay_cmds.push(format!("<prompt {}>", text)),
        TfEffect::LocalEcho { on, .. } => transcript.clay_cmds.push(format!("<localecho {}>", on)),
        TfEffect::RecordLine { text, target, .. } => transcript.clay_cmds.push(format!("<recordline {:?} {}>", target, text)),
    }
}

/// Sort a load's own returned result. Everything it did arrives as effects
/// (`record_effect`); what's left here is a control result with nothing to record, or
/// a plain result from a caller that ran a single command.
fn record_result(transcript: &mut Transcript, result: TfCommandResult) {
    let mut effects = Vec::new();
    super::effects::push_result_effects(result, &mut effects);
    for effect in effects {
        record_effect(transcript, effect);
    }
}

/// `$TFLIBDIR` if set and a real directory, else the system TinyFugue library
/// (Debian's `tf5` package installs it at `/usr/share/tf5/tf-lib`), else
/// `None`. A `;; requires-lib` case is skipped rather than failed when this
/// is `None` - the library itself never ships in this repo (see the plan's
/// "Fixtures" decision).
fn tf_lib_dir() -> Option<String> {
    super::default_tflibdir()
}

/// Leading `;; directive` comment lines at the top of a case file (before the
/// first real command). Recognised directives: `requires-lib` and
/// `preload: <file>` (see `preload_directives`).
fn case_directives(path: &Path) -> Vec<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    content
        .lines()
        .take_while(|line| {
            let t = line.trim();
            t.is_empty() || t.starts_with(";;") || t.starts_with(';')
        })
        .filter_map(|line| line.trim().strip_prefix(";;").map(|d| d.trim().to_string()))
        .collect()
}

/// `;; preload: <file>` directives from `case_directives`' output, in the
/// order they appear (a case may repeat the directive to preload more than
/// one library file). `<file>` is a bare filename resolved against the
/// resolved `tf_lib_dir()`, exactly like `/require`'s own bare-filename
/// search - see `apply_preloads`.
fn preload_directives(directives: &[String]) -> Vec<String> {
    directives
        .iter()
        .filter_map(|d| d.strip_prefix("preload:").map(|f| f.trim().to_string()))
        .collect()
}

/// Loads each of `preloads` from `lib_dir` into `engine`, quietly, via the
/// same `load_file_internal` a real `/require` would use - before the case
/// file itself runs. This directive is a Clay-test-harness-only convenience
/// (see the `;; preload:` section of `tests/tf/README.md`): real `tf`
/// already has its whole stdlib loaded by the time any script file runs, so
/// it never needs to re-load `stdlib.tf` (or another library file) itself,
/// and treats the `;; preload: ...` line as an ordinary comment. This lets a
/// case exercise stdlib/library macros without depending on the engine gap
/// tracked as finding C.2 (`/require`'s bare-filename search).
///
/// A load error is folded into `errors`, prefixed `"preload: "`, rather than
/// aborting the case - the case's own probes still run (and will usually
/// fail for a related, more specific reason of their own, e.g. `ismacro()`
/// coming back false for a macro the preload was supposed to define).
fn apply_preloads(engine: &mut TfEngine, lib_dir: &str, preloads: &[String], errors: &mut Vec<String>) {
    for file in preloads {
        let full_path = format!("{}/{}", lib_dir, file);
        let _ = super::builtins::load_file_internal(engine, &full_path, true);
        for effect in engine.take_effects() {
            if let super::effects::TfEffect::Error { msg, at } = effect {
                match at {
                    Some(at) => errors.push(format!("preload: {}: {}", at, msg)),
                    None => errors.push(format!("preload: {}", msg)),
                }
            }
        }
    }
}

/// One entry from `tests/tf/xfail.txt`: `case-name | substring-of-failure`.
struct XfailEntry {
    case: String,
    substring: String,
}

fn xfail_ledger_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tf/xfail.txt")
}

fn load_xfail_ledger() -> Vec<XfailEntry> {
    let content = fs::read_to_string(xfail_ledger_path()).unwrap_or_default();
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (case, substring) = line.split_once('|')?;
            Some(XfailEntry {
                case: case.trim().to_string(),
                substring: substring.trim().to_string(),
            })
        })
        .collect()
}

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tf/cases")
}

/// Every `tests/tf/cases/*.tf` fixture, sorted by name.
fn discover_cases() -> Vec<PathBuf> {
    let dir = cases_dir();
    let mut cases: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", dir.display(), e))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("tf"))
        .collect();
    cases.sort();
    cases
}

/// Trim trailing whitespace from each line and drop trailing blank lines, so
/// a `.expected` file's final newline (or lack of one) never causes a
/// spurious mismatch.
fn normalize_lines(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = lines.iter().map(|l| l.trim_end().to_string()).collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Like `normalize_lines`, but ALSO strips ANSI/SGR escape sequences -
/// applied only to Clay's own `transcript.echoed` text, never to
/// `.expected`/live-oracle text.
///
/// `tools/tf-oracle.sh` strips every CSI sequence from real tf's raw
/// capture before writing a `.expected` file (see its own filtering doc
/// comment) - a case that legitimately drives colored `/echo` output (e.g.
/// testcolor.tf's own "{n}"-reset-then-color idiom) never has any color
/// code left in its `.expected`. Clay's `echoed` text is never put through
/// that same stripping, so comparing it as-is against an ANSI-free
/// `.expected` is not an apples-to-apples comparison: it reports a false
/// divergence at the exact position of every real, correctly-produced
/// color code, rather than only at an actual behavioural difference (this
/// is what "echoed ...########\x1b[0m..., expected ...########..." in
/// lib_testcolor's old xfail.txt entry actually was - Clay's "{n}" reset
/// was correct, matching testcolor.tf's own script; the oracle side had
/// just already had its own, equally real reset code stripped away by
/// `tf-oracle.sh`). Stripping only the `echoed` side (not `.expected`/the
/// live oracle capture, which have no ANSI to strip - a no-op there would
/// also silently mask a genuine oracle-side regression if one ever
/// reintroduced ANSI into a `.expected` file) restores the fair comparison
/// deliberately and visibly, rather than leaving color-emitting cases
/// permanently mismatched or narrowly patching around one line.
fn normalize_echoed_lines(lines: &[String]) -> Vec<String> {
    let stripped: Vec<String> = lines.iter().map(|l| crate::util::strip_ansi_codes(l)).collect();
    normalize_lines(&stripped)
}

fn read_expected(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .collect()
}

/// A human-readable description of the first place `echoed` and `expected`
/// diverge, or `None` if they're equal.
fn describe_mismatch(echoed: &[String], expected: &[String]) -> Option<String> {
    if echoed == expected {
        return None;
    }
    for (i, (a, b)) in echoed.iter().zip(expected.iter()).enumerate() {
        if a != b {
            return Some(format!(
                "line {}: echoed {:?}, expected {:?}",
                i + 1,
                a,
                b
            ));
        }
    }
    Some(format!(
        "echoed has {} line(s), expected has {} line(s)\n  echoed:   {:?}\n  expected: {:?}",
        echoed.len(),
        expected.len(),
        echoed,
        expected
    ))
}

enum CaseStatus {
    Pass,
    Xfail,
    Skip(String),
    Fail(String),
}

fn run_case(path: &Path) -> (String, CaseStatus) {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let directives = case_directives(path);
    let preloads = preload_directives(&directives);
    let requires_lib = directives.iter().any(|d| d == "requires-lib") || !preloads.is_empty();

    let lib_dir = if requires_lib {
        match tf_lib_dir() {
            Some(dir) => Some(dir),
            None => {
                return (
                    name,
                    CaseStatus::Skip(
                        "requires-lib: $TFLIBDIR is unset and /usr/share/tf5/tf-lib does not exist"
                            .to_string(),
                    ),
                );
            }
        }
    } else {
        None
    };

    let mut engine = TfEngine::new();
    if let Some(dir) = &lib_dir {
        engine.set_global("TFLIBDIR", TfValue::String(dir.clone()));
    }

    let mut preload_errors: Vec<String> = Vec::new();
    if let Some(dir) = &lib_dir {
        apply_preloads(&mut engine, dir, &preloads, &mut preload_errors);
    }
    let mut transcript = run_script(&mut engine, path);
    if !preload_errors.is_empty() {
        preload_errors.extend(transcript.errors);
        transcript.errors = preload_errors;
    }

    let expected_path = path.with_extension("expected");
    let expected = normalize_lines(&read_expected(&expected_path));
    let echoed = normalize_echoed_lines(&transcript.echoed);

    let passes = echoed == expected && transcript.errors.is_empty();

    let ledger = load_xfail_ledger();
    let xfail_entry = ledger.iter().find(|e| e.case == name);

    if passes {
        return match xfail_entry {
            Some(entry) => (
                name.clone(),
                CaseStatus::Fail(format!(
                    "case {} passes: remove it from tests/tf/xfail.txt (was xfailed for: {:?})",
                    name, entry.substring
                )),
            ),
            None => (name, CaseStatus::Pass),
        };
    }

    let mut failure_text = String::new();
    if !transcript.errors.is_empty() {
        failure_text.push_str(&transcript.errors.join("\n"));
    }
    if let Some(desc) = describe_mismatch(&echoed, &expected) {
        if !failure_text.is_empty() {
            failure_text.push('\n');
        }
        failure_text.push_str(&desc);
    }
    if failure_text.is_empty() {
        // Shouldn't happen (passes was false), but keep the report meaningful.
        failure_text.push_str("case failed for an unrecorded reason");
    }

    match xfail_entry {
        Some(entry) if failure_text.contains(&entry.substring) => (name, CaseStatus::Xfail),
        Some(entry) => (
            name.clone(),
            CaseStatus::Fail(format!(
                "case {} is xfailed expecting substring {:?}, but the real failure was:\n{}",
                name, entry.substring, failure_text
            )),
        ),
        None => (name, CaseStatus::Fail(failure_text)),
    }
}

/// Runs every `tests/tf/cases/*.tf` fixture, printing one PASS/XFAIL/SKIP/FAIL
/// status line per case, and panics at the end with every failure's detail
/// (not just the first). Set `TF_SCRIPT_CASE=<name>` to run a single case.
#[test]
fn tf_script_cases() {
    let mut cases = discover_cases();
    assert!(
        !cases.is_empty(),
        "no fixtures found under {}",
        cases_dir().display()
    );

    if let Ok(only) = env::var("TF_SCRIPT_CASE") {
        cases.retain(|p| p.file_stem().map(|s| s == only.as_str()).unwrap_or(false));
        assert!(!cases.is_empty(), "TF_SCRIPT_CASE={:?} matched no case", only);
    }

    let mut failures: Vec<String> = Vec::new();
    for path in &cases {
        let (name, status) = run_case(path);
        match &status {
            CaseStatus::Pass => println!("PASS  {}", name),
            CaseStatus::Xfail => println!("XFAIL {}", name),
            CaseStatus::Skip(reason) => println!("SKIP  {} ({})", name, reason),
            CaseStatus::Fail(_) => println!("FAIL  {}", name),
        }
        if let CaseStatus::Fail(detail) = status {
            failures.push(format!("{}:\n{}", name, detail));
        }
    }

    if !failures.is_empty() {
        panic!(
            "tf_script_cases: {} of {} case(s) failed:\n\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n\n")
        );
    }
}

/// Path to the oracle script (`tools/tf-oracle.sh`), relative to the repo
/// checkout the test is running from.
fn oracle_script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/tf-oracle.sh")
}

/// True iff `tf` resolves on `PATH`. Checked with `which` rather than
/// running `tf` itself - `tf` with no script argument starts an interactive
/// session and would hang waiting on a terminal.
fn tf_on_path() -> bool {
    Command::new("which")
        .arg("tf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Live-diffs every non-xfailed case's Clay output against real TinyFugue's
/// output for the same file, run fresh through `tools/tf-oracle.sh` (as
/// opposed to `tf_script_cases`, which compares against the checked-in
/// `.expected` snapshot). Skips cleanly - rather than failing - when `tf`
/// isn't installed, since this test has nothing to grade Clay against
/// without it. A case listed in `tests/tf/xfail.txt` is a *known* divergence
/// already tracked there, so it's skipped here too rather than reported as a
/// mismatch.
#[test]
fn tf_script_oracle_diff() {
    if !tf_on_path() {
        println!("SKIP: tf not on PATH");
        return;
    }

    let oracle_script = oracle_script_path();
    assert!(
        oracle_script.is_file(),
        "missing oracle script at {}",
        oracle_script.display()
    );

    let cases = discover_cases();
    assert!(
        !cases.is_empty(),
        "no fixtures found under {}",
        cases_dir().display()
    );
    let ledger = load_xfail_ledger();

    let mut mismatches: Vec<String> = Vec::new();

    for path in &cases {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        if ledger.iter().any(|e| e.case == name) {
            println!("SKIP  {} (xfail - known divergence)", name);
            continue;
        }

        let directives = case_directives(path);
        let preloads = preload_directives(&directives);
        let requires_lib = directives.iter().any(|d| d == "requires-lib") || !preloads.is_empty();
        let lib_dir = if requires_lib { tf_lib_dir() } else { None };
        if requires_lib && lib_dir.is_none() {
            println!(
                "SKIP  {} (requires-lib: $TFLIBDIR unset and /usr/share/tf5/tf-lib missing)",
                name
            );
            continue;
        }

        let output = Command::new(&oracle_script).arg(path).output();
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                println!("FAIL  {}", name);
                mismatches.push(format!(
                    "{}: failed to run {}: {}",
                    name,
                    oracle_script.display(),
                    e
                ));
                continue;
            }
        };
        if !output.status.success() {
            println!("FAIL  {}", name);
            mismatches.push(format!(
                "{}: tf-oracle.sh exited with {:?}\nstderr:\n{}",
                name,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut oracle_lines: Vec<String> = stdout.lines().map(|s| s.to_string()).collect();
        let header = format!("== {}", name);
        if oracle_lines.first().is_some_and(|l| *l == header) {
            oracle_lines.remove(0);
        }
        let oracle_lines = normalize_lines(&oracle_lines);

        let mut engine = TfEngine::new();
        if let Some(dir) = &lib_dir {
            engine.set_global("TFLIBDIR", TfValue::String(dir.clone()));
        }
        let mut preload_errors: Vec<String> = Vec::new();
        if let Some(dir) = &lib_dir {
            apply_preloads(&mut engine, dir, &preloads, &mut preload_errors);
        }
        let transcript = run_script(&mut engine, path);
        let echoed = normalize_echoed_lines(&transcript.echoed);

        if echoed == oracle_lines {
            println!("PASS  {}", name);
            continue;
        }

        println!("FAIL  {}", name);
        let first_diff = echoed
            .iter()
            .zip(oracle_lines.iter())
            .enumerate()
            .find(|(_, (a, b))| a != b);
        let detail = match first_diff {
            Some((i, (a, b))) => format!(
                "{}: first mismatch at line {}: clay={:?} tf={:?}",
                name,
                i + 1,
                a,
                b
            ),
            None => format!(
                "{}: line count differs: clay has {} line(s), tf has {} line(s)\n  clay: {:?}\n  tf:   {:?}",
                name,
                echoed.len(),
                oracle_lines.len(),
                echoed,
                oracle_lines
            ),
        };
        mismatches.push(detail);
    }

    if !mismatches.is_empty() {
        panic!(
            "tf_script_oracle_diff: {} case(s) diverge from real TinyFugue:\n\n{}",
            mismatches.len(),
            mismatches.join("\n\n")
        );
    }
}

/// Smoke test for the runner itself, independent of the fixture directory:
/// a one-line inline script through `run_script` should echo exactly what it
/// `/echo`s.
#[test]
fn tf_script_runner_smoke() {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "clay_tf_script_runner_smoke_{}_{}.tf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    fs::write(&path, "/echo hello\n/quit\n").expect("write smoke-test fixture");

    let mut engine = TfEngine::new();
    let transcript = run_script(&mut engine, &path);
    let _ = fs::remove_file(&path);

    assert_eq!(transcript.echoed, vec!["hello".to_string()]);
    assert!(transcript.errors.is_empty(), "unexpected errors: {:?}", transcript.errors);
}

/// Finding 22: a file's successfully-echoed lines must survive a LATER line
/// erroring - TF interleaves output and errors rather than discarding
/// everything the file already printed. Two /echo lines, then one bad
/// command, then a third /echo that never runs (the bad line still aborts
/// the load - only the *ordering* of what came before it changes here, not
/// whether loading continues past an error).
#[test]
fn tf_script_finding_22_echoed_lines_survive_a_later_error() {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "clay_tf_script_finding22_{}_{}.tf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    // "/undef nosuchmacro" used to be this fixture's example erroring line, but
    // finding 25 (Job 13) made a missing /undef target a plain diagnostic
    // message instead of a `TfCommandResult::Error` (matching real tf, which
    // is silent on success and never treats this as load-aborting) - so it no
    // longer exercises finding 22 at all. "/set 123bad=x" (an invalid
    // variable name) still genuinely errors and is untouched by this job.
    fs::write(&path, "/echo one\n/echo two\n/set 123bad=x\n/quit\n").expect("write finding-22 fixture");

    let mut engine = TfEngine::new();
    let transcript = run_script(&mut engine, &path);
    let _ = fs::remove_file(&path);

    assert_eq!(transcript.echoed, vec!["one".to_string(), "two".to_string()]);
    // One error, reported where it happened (line 3), as the line ran.
    assert_eq!(transcript.errors.len(), 1, "errors: {:?}", transcript.errors);
    assert!(transcript.errors[0].contains(", line 3: ") && transcript.errors[0].contains("123bad"),
        "errors: {:?}", transcript.errors);
}
