//! TinyFugue's own help file (`tf-help`), for `/help` topics Clay has no help of its own
//! for. The file is TF's: a section starts with one or more `&<topic>` lines and runs to
//! the next; inside one, `#<topic>` lines start a sub-topic. Shown as TF shows it
//! (checked against real tf): "Help on: <topic>" and the section, or for a sub-topic
//! "Help on: <section>: <topic>", its part, and where to read the rest.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::TfEngine;

/// The help file: %TFHELP, else `tf-help` in %TFLIBDIR - when it exists.
pub fn help_file(engine: &TfEngine) -> Option<PathBuf> {
    let var = |name: &str| engine.get_var(name).map(|v| v.to_string_value()).filter(|s| !s.is_empty());
    if let Some(file) = var("TFHELP") {
        let path = PathBuf::from(engine.expand_tilde(&file));
        if path.is_file() {
            return Some(path);
        }
    }
    let path = Path::new(&engine.expand_tilde(&var("TFLIBDIR")?)).join("tf-help");
    path.is_file().then_some(path)
}

struct HelpFile {
    lines: Vec<String>,
}

/// The file last read: its path, when it was modified, its lines.
type Cached = (PathBuf, Option<SystemTime>, Arc<HelpFile>);

/// The file's lines, read once per change of the file.
fn load(path: &Path) -> Option<Arc<HelpFile>> {
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let mut cache = CACHE.lock().ok()?;
    if let Some((cached_path, cached_time, file)) = cache.as_ref() {
        if cached_path == path && *cached_time == modified {
            return Some(file.clone());
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let file = Arc::new(HelpFile { lines: text.lines().map(str::to_string).collect() });
    *cache = Some((path.to_path_buf(), modified, file.clone()));
    Some(file)
}

/// The run of `marker` lines around `index`: its first line and one past its last.
fn group(lines: &[String], index: usize, marker: char) -> (usize, usize) {
    let mut start = index;
    while start > 0 && lines[start - 1].starts_with(marker) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < lines.len() && lines[end].starts_with(marker) {
        end += 1;
    }
    (start, end)
}

/// TF's help on `topic`, or None. As in TF: `/topic` is tried before `topic` (so
/// `/help sub` is /sub, not the %sub variable), case matters, and the text is the file's
/// own - a section runs to the next `&` line without its `#` lines, a sub-topic to the
/// next `#` or `&` line, then says which section it came from.
pub fn lookup(engine: &TfEngine, topic: &str) -> Option<String> {
    let file = load(&help_file(engine)?)?;
    let lines = &file.lines;
    let topic = topic.trim();
    if topic.is_empty() {
        return None;
    }
    let find = |name: &str| lines.iter().position(|l| {
        (l.starts_with('&') || l.starts_with('#')) && &l[1..] == name
    });
    let slashed = (!topic.starts_with('/')).then(|| find(&format!("/{}", topic))).flatten();
    let index = slashed.or_else(|| find(topic))?;
    let marker = if lines[index].starts_with('&') { '&' } else { '#' };
    let (first, after) = group(lines, index, marker);
    // A group's last name is the one TF shows.
    let name = &lines[after - 1][1..];
    let mut out = Vec::new();
    if marker == '&' {
        out.push(format!("Help on: {}", name));
        out.extend(lines[after..].iter()
            .take_while(|l| !l.starts_with('&'))
            .filter(|l| !l.starts_with('#'))
            .cloned());
    } else {
        let section_end = (0..first).rev().find(|&i| lines[i].starts_with('&'))?;
        let section = &lines[section_end][1..];
        out.push(format!("Help on: {}: {}", section, name));
        out.extend(lines[after..].iter()
            .take_while(|l| !l.starts_with('#') && !l.starts_with('&'))
            .cloned());
        out.push(format!("For more complete information, see \"{}\".", section));
    }
    // Ended with a newline, so a closing blank line in the file is shown, as TF does.
    Some(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TF's formats (checked against real tf 5.0b8): a section by its `&` name, a `#`
    /// sub-topic with its section named, `/name` before `name`, and nothing for a name
    /// in the wrong case.
    #[test]
    fn test_lookup_formats() {
        let dir = std::env::temp_dir().join(format!("clay_tfhelp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tf-help");
        std::fs::write(&file, "&/beep\n\n/beep\n\n  Usage: /BEEP\n#hidden\n  more\n\n\
            &special variable\n&special variables\n\nintro\n#kecho\n#%kecho\n  kecho=off\n\
            \x20     echoes keys\n\n#sub\n  sub=off\n#/\n  slash\n&/sub\n\n/sub\n").unwrap();
        let mut engine = TfEngine::new();
        engine.set_global("TFHELP", super::super::TfValue::String(file.display().to_string()));
        assert_eq!(lookup(&engine, "/beep").unwrap(), "Help on: /beep\n\n/beep\n\n  Usage: /BEEP\n  more\n\n");
        assert_eq!(lookup(&engine, "beep").unwrap().lines().next(), Some("Help on: /beep"));
        assert_eq!(lookup(&engine, "kecho").unwrap(), "Help on: special variables: %kecho\n  kecho=off\n\
            \x20     echoes keys\n\nFor more complete information, see \"special variables\".\n");
        assert_eq!(lookup(&engine, "sub").unwrap(), "Help on: /sub\n\n/sub\n", "/sub wins over #sub");
        assert_eq!(lookup(&engine, "%kecho").unwrap().lines().next(), Some("Help on: special variables: %kecho"));
        assert!(lookup(&engine, "BEEP").is_none(), "case matters, as in TF");
        assert!(lookup(&engine, "nosuch").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
