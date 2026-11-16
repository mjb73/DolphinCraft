//! `--pick`: the window answering a file dialog, and the backend behind the
//! desktop portal's file chooser.
//!
//! Added feature. A file dialog is the one thing every desktop program has to
//! ask the desktop for, and on this box the answer came from
//! `xdg-desktop-portal-kde`'s own QML dialog. `--pick open|save` turns this
//! window into that dialog — the same panes, views, places and find bar, with a
//! footer that answers — and `packaging/portal/` puts it behind
//! `org.freedesktop.impl.portal.FileChooser` so a program that asks the portal
//! for a file gets this window.
//!
//! The answer is printed on stdout, one absolute path per line, and the process
//! leaves: 0 when something was chosen, 1 when the dialog was put away. That is
//! the contract the portal backend reads, and it is also usable from a shell:
//!
//! ```sh
//! file=$(dolphincraft --pick open --dir ~/Documents)
//! ```

use std::path::PathBuf;

/// What the dialog is asking for. The portal's "select a folder" is
/// `Mode::Open` with `folder` set, which is why it is not a third mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Open,
    Save,
}

/// One row of the dialog's filter: the label a person reads and the name
/// patterns the row keeps. The portal passes these through as the program that
/// asked wrote them, so nothing here may assume a particular set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub label: String,
    pub patterns: Vec<String>,
}

/// What the answer was, once the dialog is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    /// Paths were printed (or, for a save dialog, one path was).
    Chosen,
    /// The dialog was put away: nothing is printed and the exit status is 1.
    Cancelled,
}

#[derive(Clone)]
pub struct Pick {
    pub mode: Mode,
    /// More than one path may be answered. Off unless asked: the portal's own
    /// default is one file.
    pub multi: bool,
    /// A folder is the answer rather than a file (`directory` in the portal's
    /// options). A save dialog with this set asks for a folder to make.
    pub folder: bool,
    pub filters: Vec<Filter>,
    /// Which of `filters` is showing. Only ever an index that exists.
    pub active: usize,
    /// The save dialog's name field.
    pub name: String,
    /// Whether the person has typed in the name field. Until they have, marking
    /// a file in the list puts that file's name in the field, which is what
    /// every other save dialog does.
    pub name_touched: bool,
    /// The words in the bar's left-hand end, from the program that asked.
    pub title: String,
    /// The accept button's own word: "Open", "Save", "Select Folder".
    pub accept: String,
}

impl Pick {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            multi: false,
            folder: false,
            filters: Vec::new(),
            active: 0,
            name: String::new(),
            name_touched: false,
            title: if mode == Mode::Save {
                "Save a file".to_string()
            } else {
                "Open a file".to_string()
            },
            accept: match mode {
                Mode::Save => "Save".to_string(),
                Mode::Open => "Open".to_string(),
            },
        }
    }

    /// A folder answer, for the portal's `directory` request.
    pub fn for_folder(mut self, name: String) -> Self {
        self.folder = true;
        self.title = name;
        self
    }

    /// The patterns the listing is narrowed to, or `None` when every name is
    /// offered. The pane keeps folders whatever this says: a dialog you cannot
    /// walk through is not a dialog.
    pub fn patterns(&self) -> Option<Vec<String>> {
        if self.folder {
            return None;
        }
        let rows = if self.filters.is_empty() {
            None
        } else {
            Some(
                self.filters[self.active.min(self.filters.len() - 1)]
                    .patterns
                    .clone(),
            )
        };
        rows.filter(|list| !list.is_empty())
    }

    /// Whether a *file* of this name may be answered under the filter showing.
    pub fn keeps(&self, name: &str) -> bool {
        match self.patterns() {
            None => true,
            Some(list) => list.iter().any(|p| glob_matches(p, name)),
        }
    }

    /// The filter rows as the bar draws them, so the bar and the listing cannot
    /// disagree about which one is showing.
    pub fn filter_labels(&self) -> Vec<String> {
        self.filters.iter().map(|f| f.label.clone()).collect()
    }

    /// Words for a file the filter refuses, in the bar's own voice.
    pub fn refusal(&self, name: &str) -> String {
        let label = self
            .filters
            .get(self.active)
            .map(|f| f.label.clone())
            .unwrap_or_else(|| "this filter".to_string());
        format!("{name} is not one of the {label} files this asked for")
    }
}

/// The paths the dialog would answer with right now, or `None` when there is
/// nothing to answer. One function, so the button that is enabled, the answer a
/// double click gives and the answer Enter gives cannot disagree.
pub fn candidate(
    pick: &Pick,
    dir: &std::path::Path,
    marked: &[PathBuf],
    cursor: Option<&PathBuf>,
) -> Option<Vec<PathBuf>> {
    match pick.mode {
        Mode::Save => save_target(dir, &pick.name).map(|path| vec![path]),
        Mode::Open => {
            let mut answer: Vec<PathBuf> = marked
                .iter()
                .filter(|path| may_answer(pick, path, dir))
                .cloned()
                .collect();
            if answer.is_empty() {
                if let Some(path) = cursor {
                    if may_answer(pick, path, dir) {
                        answer.push(path.clone());
                    }
                }
            }
            // A folder request with nothing marked answers with the folder on
            // screen: the button must never be dead in a dialog.
            if answer.is_empty() && pick.folder {
                answer.push(dir.to_path_buf());
            }
            if answer.is_empty() {
                return None;
            }
            if !pick.multi {
                answer.truncate(1);
            }
            Some(answer)
        }
    }
}

/// Whether one path may be the dialog's answer. A folder request takes folders
/// and nothing else; a file request takes files the filter keeps, and never a
/// folder (double clicking one walks into it, the way it does in a file window).
pub fn may_answer(pick: &Pick, path: &std::path::Path, _dir: &std::path::Path) -> bool {
    if pick.folder {
        return path.is_dir();
    }
    if path.is_dir() || !path.is_file() {
        return false;
    }
    path.file_name()
        .map(|name| pick.keeps(&name.to_string_lossy()))
        .unwrap_or(false)
}

/// One filter row from a command-line word, as `"Label:*.png;*.jpg"`. A word
/// without a colon is a bare set of patterns ("*.png;*.jpg") and gets the label
/// the patterns themselves spell. Nothing a program passes may fail here: a
/// filter row that cannot be read is dropped, not fatal — the portal hands over
/// whatever the asking program declared.
pub fn parse_filter(word: &str) -> Option<Filter> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let (label, patterns) = match word.split_once(':') {
        Some((label, rest)) => (label.trim().to_string(), rest),
        None => (String::new(), word),
    };
    let patterns: Vec<String> = patterns
        .split([';', ','])
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if patterns.is_empty() {
        return None;
    }
    let label = if label.is_empty() {
        patterns.join(", ")
    } else {
        label
    };
    Some(Filter { label, patterns })
}

/// Does one name match one glob? `*` takes any run of characters (including
/// none), `?` exactly one, and the comparison ignores case, which is what both
/// GTK's and KIO's own patterns do.
///
/// Written as the two-pointer walk with a remembered star rather than a regex:
/// the patterns come from another program, and a dialog must not be able to
/// hang on one.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Where the last `*` was, and the text position it is currently eating up
    // to. A mismatch backs up to the star and lets it eat one more character.
    let mut star: Option<usize> = None;
    let mut resume = 0usize;
    while ti < text.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = Some(pi);
            pi += 1;
            resume = ti;
        } else if let Some(at) = star {
            pi = at + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// The paths a save dialog answers with: the folder the pane is showing and the
/// name in the field. A name that is a path in its own right ("/tmp/x.png") is
/// taken as one, which is what a person pasting a path expects.
pub fn save_target(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let typed = PathBuf::from(name);
    if typed.is_absolute() {
        return Some(typed);
    }
    Some(dir.join(typed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_keep_what_they_name() {
        assert!(glob_matches("*.png", "shot.png"));
        assert!(glob_matches("*.PNG", "shot.png"));
        assert!(glob_matches("*", "anything at all"));
        assert!(glob_matches("*.tar.*", "x.tar.gz"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(!glob_matches("*.png", "shot.jpg"));
        assert!(!glob_matches("*.png", "png"));
    }
}
