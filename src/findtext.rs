//! Added feature: *Find in Files* — search inside documents, not just names.
//!
//! The original's find bar offers three modes and names them in
//! `/usr/bin/dolphin`: `File names and contents` (string 0x22e0db) and
//! `File contents` (0x22e10c), alongside a plain name search. The file-name
//! mode is what this rebuild already had (the find bar filters names as you
//! type); the two content modes had no code behind them at all, because the
//! original hands the reading work to KIO's search helper
//! (`kio_file`/Baloo) and this window has no such helper.
//!
//! This module does the reading itself: it walks the folder, opens every text
//! file it finds, counts the phrase, and keeps the first line it appears on so
//! the window can show *where* it was written, not only *which* file holds it.
//!
//! Rules, each of which `MECHANICS.md` records:
//! * symbolic links are never followed (a link to `/` would make the walk
//!   endless and the results meaningless);
//! * hidden files and folders are left out — the same choice the original's
//!   find dialog makes by default — and the window says so in its summary;
//! * a file with a NUL byte in its first 8 KiB is treated as binary and skipped
//!   unless the person ticks *Include binary files*;
//! * a file larger than `MAX_BYTES` is not read, and the summary counts it as
//!   too big rather than pretending it held nothing;
//! * the walk stops after `MAX_FILES` files or `MAX_HITS` hits, and says it
//!   stopped, so a search in a home folder always ends.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The three modes the original's find bar offers, in its own words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Names,
    NamesAndContents,
    Contents,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Names => "File names",
            Mode::NamesAndContents => "File names and contents",
            Mode::Contents => "File contents",
        }
    }

    /// Whether a file's name alone can make it a hit.
    pub fn wants_names(self) -> bool {
        matches!(self, Mode::Names | Mode::NamesAndContents)
    }

    /// Whether the file has to be opened and read.
    pub fn wants_contents(self) -> bool {
        matches!(self, Mode::NamesAndContents | Mode::Contents)
    }
}

/// How many hits the window will hold. Past this the walk stops: a shortlist a
/// person can act on, not an inventory of every match in a home folder.
pub const MAX_HITS: usize = 300;
/// How many files are looked at before the walk gives up and says so.
pub const MAX_FILES: usize = 20000;
/// The largest file whose insides are read, in bytes. Four megabytes covers a
/// novel, a long report or a big log, and keeps a stray disk image from being
/// pulled into memory.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The first bytes examined when deciding whether a file is binary.
pub const SNIFF_BYTES: usize = 8 * 1024;
/// How much of a matching line is shown. Enough to recognise the sentence, short
/// enough to fit one row of the list.
pub const SNIPPET_CHARS: usize = 120;

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    /// 1-based line number of the first match, or 0 when only the name matched.
    pub line_no: usize,
    /// The matching line, trimmed and shortened; empty for a name-only hit.
    pub line: String,
    /// How many times the phrase appears in this file (1 for a name-only hit).
    pub matches: usize,
}

impl Hit {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn folder(&self) -> String {
        self.path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub phrase: String,
    pub mode: Mode,
    pub case_sensitive: bool,
    /// Tick box, off by default like the original's find dialog.
    pub include_binary: bool,
}

impl Options {
    pub fn new(phrase: impl Into<String>) -> Self {
        Self {
            phrase: phrase.into(),
            mode: Mode::Contents,
            case_sensitive: false,
            include_binary: false,
        }
    }
}

pub enum Msg {
    Hit(Hit),
    Progress { files: usize, matches: usize },
    Done {
        files: usize,
        matches: usize,
        skipped_big: usize,
        skipped_binary: usize,
        truncated: bool,
    },
    Failed(String),
}

/// A search in flight, and everything it has found so far.
pub struct FindText {
    pub root: PathBuf,
    pub opts: Options,
    pub hits: Vec<Hit>,
    pub files_seen: usize,
    pub matches: usize,
    pub skipped_big: usize,
    pub skipped_binary: usize,
    pub scanning: bool,
    pub done: bool,
    pub truncated: bool,
    pub error: Option<String>,
    pub started: Instant,
    pub took: Option<Duration>,
    /// Set for the first frame so the phrase field takes the keyboard without a
    /// click: the window exists to be typed into.
    pub focus: bool,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
}

impl FindText {
    pub fn start(root: PathBuf, opts: Options) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        // An empty phrase means there is nothing to look for: opening the window
        // from the menu must not walk a home folder for nothing, so the walk is
        // not started at all and the window waits for a phrase.
        if opts.phrase.trim().is_empty() {
            return Self {
                root,
                opts,
                hits: Vec::new(),
                files_seen: 0,
                matches: 0,
                skipped_big: 0,
                skipped_binary: 0,
                scanning: false,
                done: false,
                truncated: false,
                error: None,
                started: Instant::now(),
                took: None,
                focus: true,
                cancel,
                rx,
            };
        }
        let flag = cancel.clone();
        let r = root.clone();
        let o = opts.clone();
        std::thread::spawn(move || {
            let mut sent = 0usize;
            let mut files = 0usize;
            let mut matches = 0usize;
            let mut skipped_big = 0usize;
            let mut skipped_binary = 0usize;
            let mut truncated = false;
            let walker = walkdir::WalkDir::new(&r)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
                });
            for item in walker.flatten() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                if files >= MAX_FILES || sent >= MAX_HITS {
                    truncated = true;
                    break;
                }
                if !item.file_type().is_file() {
                    continue;
                }
                files += 1;
                if files % 64 == 0 {
                    let _ = tx.send(Msg::Progress { files, matches });
                }
                let path = item.path().to_path_buf();
                let name = item.file_name().to_string_lossy().to_string();
                let found = inspect(&path, &name, &o, &mut skipped_big, &mut skipped_binary);
                let Some(mut hit) = found else { continue };
                matches += hit.matches;
                hit.matches = hit.matches.max(1);
                sent += 1;
                if tx.send(Msg::Hit(hit)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Done {
                files,
                matches,
                skipped_big,
                skipped_binary,
                truncated,
            });
        });
        Self {
            root,
            opts,
            hits: Vec::new(),
            files_seen: 0,
            matches: 0,
            skipped_big: 0,
            skipped_binary: 0,
            scanning: true,
            done: false,
            truncated: false,
            error: None,
            started: Instant::now(),
            took: None,
            focus: true,
            cancel,
            rx,
        }
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Take whatever the walk has sent since the last frame.
    pub fn poll(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Hit(hit)) => self.hits.push(hit),
                Ok(Msg::Progress { files, matches }) => {
                    self.files_seen = files;
                    self.matches = matches;
                }
                Ok(Msg::Done {
                    files,
                    matches,
                    skipped_big,
                    skipped_binary,
                    truncated,
                }) => {
                    self.files_seen = files;
                    self.matches = matches;
                    self.skipped_big = skipped_big;
                    self.skipped_binary = skipped_binary;
                    self.truncated = truncated;
                    self.scanning = false;
                    self.done = true;
                    self.took = Some(self.started.elapsed());
                }
                Ok(Msg::Failed(e)) => {
                    self.error = Some(e);
                    self.scanning = false;
                    self.done = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.scanning = false;
                    break;
                }
            }
        }
    }

    /// The summary line under the fields: what was looked at, what was found,
    /// and everything the search deliberately left out.
    pub fn summary(&self) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if self.opts.phrase.trim().is_empty() {
            return "Nothing searched yet  ·  hidden folders left out".to_string();
        }
        let mut out = if self.scanning {
            format!(
                "Searching…  {} file{} looked at, {} match{} so far",
                self.files_seen,
                plural(self.files_seen),
                self.matches,
                plural(self.matches)
            )
        } else {
            format!(
                "{} file{} looked at, {} match{} in {} file{}",
                self.files_seen,
                plural(self.files_seen),
                self.matches,
                plural(self.matches),
                self.hits.len(),
                plural(self.hits.len())
            )
        };
        if let Some(took) = self.took {
            out.push_str(&format!("  ·  {:.1} s", took.as_secs_f32()));
        }
        out.push_str("  ·  hidden folders left out");
        if self.skipped_binary > 0 {
            out.push_str(&format!("  ·  {} binary file{} skipped", self.skipped_binary, plural(self.skipped_binary)));
        }
        if self.skipped_big > 0 {
            out.push_str(&format!(
                "  ·  {} file{} too large to read",
                self.skipped_big,
                plural(self.skipped_big)
            ));
        }
        if self.truncated {
            out.push_str(&format!(
                "  ·  stopped after {} hits or {} files",
                MAX_HITS, MAX_FILES
            ));
        }
        out
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Decide whether one file is a hit, and where.
///
/// Returns `None` when the file holds nothing, and `Some(hit)` otherwise. The
/// two counters are filled in so the window can say what it passed over.
pub fn inspect(
    path: &Path,
    name: &str,
    opts: &Options,
    skipped_big: &mut usize,
    skipped_binary: &mut usize,
) -> Option<Hit> {
    let name_hit = opts.mode.wants_names() && name_matches(name, &opts.phrase, opts.case_sensitive);
    if !opts.mode.wants_contents() {
        if name_hit {
            return Some(Hit {
                path: path.to_path_buf(),
                line_no: 0,
                line: String::new(),
                matches: 1,
            });
        }
        return None;
    }
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_BYTES {
        *skipped_big += 1;
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if !opts.include_binary && looks_binary(&bytes[..bytes.len().min(SNIFF_BYTES)]) {
        *skipped_binary += 1;
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let (line_no, line) = match first_match_line(&text, &opts.phrase, opts.case_sensitive) {
        Some(found) => found,
        None => (0, String::new()),
    };
    let count = count_matches(&text, &opts.phrase, opts.case_sensitive);
    if count == 0 && !name_hit {
        return None;
    }
    Some(Hit {
        path: path.to_path_buf(),
        line_no,
        line,
        matches: count,
    })
}

/// Does this file name hold the phrase the way the search asked for it?
pub fn name_matches(name: &str, phrase: &str, case_sensitive: bool) -> bool {
    if phrase.is_empty() {
        return false;
    }
    if case_sensitive {
        name.contains(phrase)
    } else {
        name.to_lowercase().contains(&phrase.to_lowercase())
    }
}

/// How many times the phrase appears in the text, counting repeats on one line
/// separately. Overlaps are not counted twice (`aa` in `aaa` is one match),
/// which is what every other search box on the desktop does.
pub fn count_matches(text: &str, phrase: &str, case_sensitive: bool) -> usize {
    if phrase.is_empty() {
        return 0;
    }
    if case_sensitive {
        text.matches(phrase).count()
    } else {
        text.to_lowercase().matches(&phrase.to_lowercase()).count()
    }
}

/// The first line that holds the phrase, as a 1-based number and the line
/// itself trimmed and shortened for one row of the list.
pub fn first_match_line(text: &str, phrase: &str, case_sensitive: bool) -> Option<(usize, String)> {
    if phrase.is_empty() {
        return None;
    }
    let needle = if case_sensitive {
        phrase.to_string()
    } else {
        phrase.to_lowercase()
    };
    for (i, line) in text.lines().enumerate() {
        let hay = if case_sensitive {
            line.to_string()
        } else {
            line.to_lowercase()
        };
        if hay.contains(&needle) {
            return Some((i + 1, snippet(line)));
        }
    }
    None
}

/// A line made fit for one row: runs of spaces and tabs collapsed, trimmed, and
/// cut to `SNIPPET_CHARS` characters with an ellipsis when it is longer.
pub fn snippet(line: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in line.chars() {
        let is_space = ch == ' ' || ch == '\t' || ch == '\r';
        if is_space {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
            continue;
        }
        last_space = false;
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    let trimmed = out.trim().to_string();
    if trimmed.chars().count() <= SNIPPET_CHARS {
        return trimmed;
    }
    let cut: String = trimmed.chars().take(SNIPPET_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// A file is treated as binary when a NUL byte appears in its first `SNIFF_BYTES`
/// bytes. That is the same crude test the desktop's own tools use, and it is
/// right for every text format this window is likely to meet.
pub fn looks_binary(head: &[u8]) -> bool {
    head.contains(&0u8)
}
