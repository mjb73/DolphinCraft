//! Finding the programs a file can be opened with.
//!
//! The original asks the desktop for this: right-clicking a file in Dolphin
//! puts the *preferred* program's name in the first row ("Open with Kate") and
//! a submenu of the other programs under it. In libKF6KIOWidgets.so.6.31.0 the
//! submenu's texts are `&Open With` @0x11c24a and `Open &with %1` @0x11c258,
//! both loaded by the function at 0x646d0; the chooser behind the last row
//! carries `&Open With` @0x11c37c and `&Other Application` @0x11c387 in the
//! function at 0x6d980, and its `&Remember application association for all
//! files of type "%1" (%2)` @0x120210 is drawn by the function at 0x834c0.
//! The preferred program comes from the desktop's own `mimeapps.list`, so
//! every program on the computer agrees about it, and that tick box is how a
//! choice gets in there.
//!
//! This module reads the same files: the `.desktop` entries under the data
//! directories, the globs that turn a file name into a type, and `mimeapps.list`
//! for the preferred program. Nothing here runs at startup; the lists are read
//! when a menu is opened, then kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// One installed program, as its `.desktop` entry describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppEntry {
    /// The entry's file name, e.g. `org.kde.kate.desktop`.
    pub id: String,
    /// The name shown in menus (`Name=`).
    pub name: String,
    /// The command line, field codes and all (`Exec=`).
    pub exec: String,
    /// Icon name (`Icon=`), used for the row's glyph.
    pub icon: String,
    /// The types it says it opens (`MimeType=`), lower-cased.
    pub mimes: Vec<String>,
    /// It wants a terminal window around it (`Terminal=true`).
    pub terminal: bool,
}

/// Read one `.desktop` entry. `None` when it is not an application, is hidden,
/// or has no command to run: the same three reasons the desktop's own index
/// drops it. Localised `Name[xx]=` rows are ignored, so the plain `Name=` wins.
pub fn parse_desktop(id: &str, text: &str) -> Option<AppEntry> {
    let mut in_entry = false;
    let mut kind = String::new();
    let mut name = String::new();
    let mut exec = String::new();
    let mut icon = String::new();
    let mut mimes = String::new();
    let mut terminal = false;
    let mut hidden = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "Type" => kind = value.to_string(),
            "Name" => name = value.to_string(),
            "Exec" => exec = value.to_string(),
            "Icon" => icon = value.to_string(),
            "MimeType" => mimes = value.to_string(),
            "Terminal" => terminal = value.eq_ignore_ascii_case("true"),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "NoDisplay" => hidden |= value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    if hidden || name.is_empty() || exec.is_empty() {
        return None;
    }
    if !kind.is_empty() && kind != "Application" {
        return None;
    }
    Some(AppEntry {
        id: id.to_string(),
        name,
        exec,
        icon,
        mimes: mimes
            .split(';')
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty())
            .collect(),
        terminal,
    })
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::home_dir().join(".local/share"));
    dirs.push(data_home.join("applications"));
    // The user's flatpak store is a data directory of its own. Some desktops put
    // it in `XDG_DATA_DIRS` and some do not, so it is added here rather than
    // hoped for: without it every application from the store disappears from
    // *Open With* and from *Create New* on a machine whose `XDG_DATA_HOME` is set
    // to something other than `~/.local/share`.
    dirs.push(data_home.join("flatpak/exports/share/applications"));
    let extra = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for part in extra.split(':').filter(|p| !p.is_empty()) {
        dirs.push(PathBuf::from(part).join("applications"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

/// Every program the desktop offers, once each, in the order the data
/// directories give them (so a user's own copy of an entry wins).
pub fn catalog() -> &'static Vec<AppEntry> {
    static CATALOG: OnceLock<Vec<AppEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut seen: Vec<String> = Vec::new();
        let mut out: Vec<AppEntry> = Vec::new();
        for dir in data_dirs() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "desktop").unwrap_or(false))
                .collect();
            files.sort();
            for path in files {
                let Some(id) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                    continue;
                };
                if seen.contains(&id) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(app) = parse_desktop(&id, &text) {
                    seen.push(id);
                    out.push(app);
                }
            }
        }
        out
    })
}

/// The programs that say they open `mime`, in name order. A program that names
/// the type in its `MimeType=` list is the desktop's whole test; the rebuild
/// asks no more than that.
pub fn openers_for(mime: &str) -> Vec<AppEntry> {
    let mut out: Vec<AppEntry> = catalog()
        .iter()
        .filter(|a| a.mimes.iter().any(|m| m == mime))
        .cloned()
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// The list the chooser shows: first the programs that say they open `mime`,
/// then every other program the desktop has, each half in name order. The
/// original's chooser leads with the programs it knows can open the type and
/// keeps the rest within reach below them; ordering by name inside each half
/// is what makes a long list findable at a glance.
pub fn chooser(mime: &str) -> Vec<AppEntry> {
    let openers = openers_for(mime);
    let mut rest: Vec<AppEntry> = catalog()
        .iter()
        .filter(|a| !openers.iter().any(|o| o.id == a.id))
        .cloned()
        .collect();
    rest.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let mut out = openers;
    out.extend(rest);
    out
}

pub fn find(id: &str) -> Option<AppEntry> {
    catalog().iter().find(|a| a.id == id).cloned()
}

// ---------------------------------------------------------------------------
// What type a file is
// ---------------------------------------------------------------------------

/// Extension → type, read from the desktop's own `globs` list, so a `.webp` is
/// `image/webp` here for the same reason it is everywhere else.
fn globs() -> &'static HashMap<String, String> {
    static GLOBS: OnceLock<HashMap<String, String>> = OnceLock::new();
    GLOBS.get_or_init(|| {
        let mut map: HashMap<String, String> = HashMap::new();
        for file in ["/usr/share/mime/globs", "/usr/local/share/mime/globs"] {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut parts = line.split(':');
                let Some(mime) = parts.next() else { continue };
                for pattern in parts {
                    let p = pattern.trim().to_ascii_lowercase();
                    if let Some(ext) = p.strip_prefix("*.") {
                        map.entry(ext.to_string()).or_insert_with(|| mime.to_string());
                    }
                }
            }
        }
        map
    })
}

/// The type of one file, from its name. A name with no extension, or one the
/// desktop's list does not know, gets the desktop's own second answer: the
/// first kilobyte is read, and a file of plain text is `text/plain`. That is
/// why a `Cargo.lock` or a `Makefile` offers editors in its menu instead of
/// nothing at all; anything with a NUL or a control byte stays
/// `application/octet-stream` — and a folder is `inode/directory`.
pub fn mime_of(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_string();
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext.is_empty() {
        return sniff_or(path, "application/octet-stream");
    }
    match globs().get(&ext).cloned() {
        Some(mime) => mime,
        None => sniff_or(path, "application/octet-stream"),
    }
}

/// `text/plain` when the file's first kilobyte looks like text, else the
/// fallback. The test is `file`'s own: a NUL byte or a control byte below a tab
/// means binary, everything else is text.
fn sniff_or(path: &Path, fallback: &str) -> String {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return fallback.to_string();
    };
    let mut buf = [0u8; 1024];
    let Ok(n) = file.read(&mut buf) else {
        return fallback.to_string();
    };
    if n == 0 {
        return fallback.to_string();
    }
    if buf[..n].iter().any(|&b| b == 0 || b < 9) {
        return fallback.to_string();
    }
    "text/plain".to_string()
}

/// The type a menu should name for a whole selection: a shared type when every
/// item has one, else the first item's, which is also what the original's
/// "Open With" does with a mixed selection.
pub fn mime_of_selection(paths: &[PathBuf]) -> String {
    let mut first: Option<String> = None;
    for p in paths {
        let m = mime_of(p);
        match &first {
            None => first = Some(m),
            Some(f) if *f == m => {}
            Some(_) => return mime_of(&paths[0]),
        }
    }
    first.unwrap_or_else(|| "application/octet-stream".to_string())
}

// ---------------------------------------------------------------------------
// The desktop's preferred program
// ---------------------------------------------------------------------------

fn mimeapps_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::home_dir().join(".config"));
    base.join("mimeapps.list")
}

/// The entry named as the default for `mime` in `mimeapps.list`, if any. The
/// file is read on each call because it is small and because another program
/// may have changed it since the last menu; the parse is cached on the file's
/// own stamp.
pub fn default_for(mime: &str) -> Option<String> {
    let path = mimeapps_path();
    let text = std::fs::read_to_string(&path).ok()?;
    parse_mimeapps(&text)
        .get(mime)
        .and_then(|ids| ids.first().cloned())
}

/// `[Default Applications]` as type → programs, in the order the file lists
/// them. Lines before any section header belong to `[Default Applications]`,
/// which is what the specification says.
pub fn parse_mimeapps(text: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_defaults = true;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]";
            continue;
        }
        if !in_defaults || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let ids: Vec<String> = value
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !ids.is_empty() {
            out.insert(key.trim().to_ascii_lowercase(), ids);
        }
    }
    out
}

/// The same file with `mime` now opening in `id`, everything else untouched.
/// Only the one key is rewritten, so hand-written rows and other sections keep
/// their place and their order.
pub fn with_default(text: &str, mime: &str, id: &str) -> String {
    let mime = mime.to_ascii_lowercase();
    let new_row = format!("{}={}", mime, id);
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut section: Option<String> = None;
    let mut insert_at: Option<usize> = None;
    let mut defaults_end: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            if section.as_deref() == Some("Default Applications") && defaults_end.is_none() {
                defaults_end = Some(i);
            }
            section = Some(t.trim_start_matches('[').trim_end_matches(']').to_string());
            continue;
        }
        if section.is_none() || section.as_deref() == Some("Default Applications") {
            if let Some((key, _)) = t.split_once('=') {
                if key.trim().eq_ignore_ascii_case(&mime) {
                    lines[i] = new_row.clone();
                    return finish(lines);
                }
            }
            if section.as_deref() == Some("Default Applications") {
                insert_at = Some(i + 1);
            }
        }
    }
    match (insert_at, defaults_end) {
        (Some(at), _) => lines.insert(at, new_row),
        (None, Some(end)) => lines.insert(end, new_row),
        _ => {
            if !lines.is_empty() && lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push("[Default Applications]".to_string());
            lines.push(new_row);
        }
    }
    finish(lines)
}

fn finish(mut lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    lines.clear();
    text
}

/// Remember that `mime` opens in `id`. Returns the file it wrote.
pub fn remember(mime: &str, id: &str) -> Result<PathBuf, String> {
    let path = mimeapps_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = with_default(&text, mime, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Running a program
// ---------------------------------------------------------------------------

/// Turn the stored `Exec=` line into the arguments one launch needs, with the
/// desktop entry field codes filled in. The specification's codes are `%f`
/// (one file), `%F` (all of them), `%u`/`%U` (the same as URLs), `%i` (icon),
/// `%c` (name), `%k` (the entry's own path) and `%%` (a literal per cent sign);
/// anything else is left out rather than passed through as garbage.
pub fn command_line(exec: &str, paths: &[PathBuf], name: &str, icon: &str, desk: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut had_file_code = false;
    for token in split_command(exec) {
        let mut out = String::new();
        let mut chars = token.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('f') => {
                    had_file_code = true;
                    if let Some(p) = paths.first() {
                        out.push_str(&p.to_string_lossy());
                    }
                }
                Some(code @ ('f' | 'F' | 'u' | 'U')) => {
                    had_file_code = true;
                    let as_url = matches!(code, 'u' | 'U');
                    let joined: Vec<String> = if as_url {
                        paths.iter().map(|p| file_url(p)).collect()
                    } else {
                        paths.iter().map(|p| p.to_string_lossy().to_string()).collect()
                    };
                    match joined.split_first() {
                        None => out.clear(),
                        Some((first, rest)) => {
                            out.push_str(first);
                            for extra in rest {
                                args.push(extra.clone());
                            }
                        }
                    }
                }
                Some('i') => {
                    if !icon.is_empty() {
                        args.push("--icon".to_string());
                        args.push(icon.to_string());
                    }
                }
                Some('c') => out.push_str(name),
                Some('k') => out.push_str(desk),
                Some('%') => out.push('%'),
                Some(_) => {}
                None => out.push('%'),
            }
        }
        if !out.is_empty() {
            args.push(out);
        }
    }
    if !had_file_code {
        for p in paths {
            args.push(p.to_string_lossy().to_string());
        }
    }
    args
}

/// `file:///path/to/thing`, with anything outside the URL-safe set escaped.
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Split an `Exec=` line on spaces, keeping quoted stretches and backslash
/// escapes together — the rule the desktop entry specification gives.
pub fn split_command(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' if quote.is_none() => quote = Some(c),
            '"' | '\'' if quote == Some(c) => quote = None,
            '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if quote.is_none() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Start one program on these files. Detached: the window does not wait for it
/// and does not read its output.
pub fn launch(app: &AppEntry, paths: &[PathBuf]) -> Result<(), String> {
    let desk = catalog()
        .iter()
        .position(|a| a.id == app.id)
        .map(|_| app.id.clone())
        .unwrap_or_default();
    let args = command_line(&app.exec, paths, &app.name, &app.icon, &desk);
    if args.is_empty() {
        return Err(format!("{} has no program to run.", app.name));
    }
    let (program, rest) = terminal_wrap(app.terminal, &args);
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&rest);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start {}: {}", app.name, e))
}

/// A program that asks for a terminal gets one, the way the entry's
/// `Terminal=true` means.
fn terminal_wrap(terminal: bool, args: &[String]) -> (String, Vec<String>) {
    if !terminal {
        return (args[0].clone(), args[1..].to_vec());
    }
    for (term, flag) in [
        ("konsole", "--hold -e"),
        ("x-terminal-emulator", "-e"),
        ("gnome-terminal", "--"),
        ("xterm", "-e"),
    ] {
        if which(term) {
            let mut rest: Vec<String> = flag.split_whitespace().map(|s| s.to_string()).collect();
            rest.extend(args.iter().cloned());
            return (term.to_string(), rest);
        }
    }
    (args[0].clone(), args[1..].to_vec())
}

pub(crate) fn which(name: &str) -> bool {
    std::env::var("PATH")
        .map(|path| {
            path.split(':')
                .any(|dir| Path::new(dir).join(name).is_file())
        })
        .unwrap_or(false)
}

/// A glyph for a row, chosen from the icon name the entry asked for. The
/// rebuild draws its icons in code, so the names only pick a kind.
pub fn glyph_for(icon: &str) -> &'static str {
    let i = icon.to_ascii_lowercase();
    if i.contains("image") || i.contains("photo") || i.contains("gwenview") || i.contains("gimp") {
        "picture"
    } else if i.contains("video") || i.contains("vlc") || i.contains("mpv") {
        "play"
    } else if i.contains("music") || i.contains("audio") {
        "audio"
    } else if i.contains("pdf") || i.contains("document") || i.contains("writer") {
        "doc"
    } else if i.contains("text") || i.contains("kate") || i.contains("editor") {
        "text"
    } else if i.contains("browser") || i.contains("web") {
        "globe"
    } else {
        "app"
    }
}

/// The sentence on the chooser's tick box. The original words it
/// `&Remember application association for all files of type "%1" (%2)`, where
/// %1 is the name of the type and %2 the type itself (0x120210 and 0x120258 in
/// libKF6KIOWidgets.so.6); this build names the extension first, because that
/// is what the person recognises in a file name.
pub fn remember_label(ext: &str, mime: &str) -> String {
    if ext.is_empty() {
        format!("Remember this choice for all files of type \"{}\"", mime)
    } else {
        format!(
            "Remember this choice for all files of type \"{}\" ({})",
            ext, mime
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_entry_is_read() {
        let text = "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate %U\nIcon=kate\nMimeType=text/plain;text/markdown;\nTerminal=false\n";
        let app = parse_desktop("org.kde.kate.desktop", text).expect("an application");
        assert_eq!(app.name, "Kate");
        assert_eq!(app.exec, "kate %U");
        assert_eq!(app.mimes, vec!["text/plain", "text/markdown"]);
        assert!(!app.terminal);
    }

    #[test]
    fn a_hidden_entry_is_not_offered() {
        let text = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert!(parse_desktop("x.desktop", text).is_none());
    }

    #[test]
    fn a_remembered_choice_rewrites_only_that_row() {
        let before = "[Default Applications]\ntext/plain=gedit.desktop\nimage/png=shotwell.desktop\n\n[Added Associations]\ntext/plain=other.desktop\n";
        let after = with_default(before, "text/plain", "org.kde.kate.desktop");
        assert!(after.contains("text/plain=org.kde.kate.desktop"));
        assert!(after.contains("image/png=shotwell.desktop"));
        assert!(after.contains("[Added Associations]\ntext/plain=other.desktop"));
    }

    #[test]
    fn a_remembered_choice_creates_the_section_when_absent() {
        let after = with_default("", "text/plain", "org.kde.kate.desktop");
        assert_eq!(after, "[Default Applications]\ntext/plain=org.kde.kate.desktop\n");
    }

    #[test]
    fn field_codes_are_filled_in() {
        let p = PathBuf::from("/tmp/a b.txt");
        let args = command_line("editor %F --name %c", &[p], "Editor", "", "");
        assert_eq!(args[0], "editor");
        assert!(args.contains(&"/tmp/a b.txt".to_string()));
        assert!(args.contains(&"--name".to_string()));
        assert!(args.contains(&"Editor".to_string()));
    }

    #[test]
    fn a_command_with_no_file_code_gets_the_files_appended() {
        let args = command_line("gwenview", &[PathBuf::from("/tmp/x.png")], "Viewer", "", "");
        assert_eq!(args, vec!["gwenview", "/tmp/x.png"]);
    }

    #[test]
    fn urls_are_escaped() {
        assert_eq!(
            file_url(Path::new("/tmp/a b.png")),
            "file:///tmp/a%20b.png"
        );
    }
}
