//! Added feature: copying and moving between the two views, and into a folder
//! of the person's choosing.
//!
//! The original program has both halves of this already:
//!
//! * **Copy to Other View** / **Move to Other View** — two actions named
//!   `copyToInactiveSplitView` and `moveToInactiveSplitView`, labelled *Copy to
//!   Other View* (string 0x22d6c9) and *Move to Other View* (0x22d707), with the
//!   default shortcuts **Shift+F5** and **Shift+F6** (two `QKeySequence`
//!   constants 0x03000034 and 0x03000035 handed to
//!   `KActionCollection::setDefaultShortcut` at 0x67436 and 0x676b5 in
//!   `/usr/bin/dolphin`).
//! * **Copy To…** / **Move To…** in the right-click menu, installed by KIO's
//!   `KFileCopyToMenu`; the original only carries the settings page that turns
//!   them on, titled *'Copy To' and 'Move To' commands* (0x236a20).
//!
//! This rebuild had the first pair as an empty stub (`Request::CopyToOther`
//! answered `let _ = cut;`) and neither of the rows, so this module is the
//! engine both of them now use, and the second pair is the addition on top.
//!
//! The rules are the original's own, read out of `DolphinMainWindow::updateActions`
//! (`FUN_00179450` @0x179450, `/usr/bin/dolphin`) — the order the four refusals
//! are tested in, and their exact wording. One thing is deliberately *not* the
//! original's: where the original hands the job to KIO and waits for the
//! "File Already Exists" dialog (whose *Overwrite* would replace the file
//! already there), this version never overwrites anything — the newcomer gets
//! the next free name instead, the same way a paste does.

use std::path::{Path, PathBuf};

/// The action's label, exactly as the original sets it (`0x22d6c9`).
pub const COPY_LABEL: &str = "Copy to Other View";
/// `0x22d707`.
pub const MOVE_LABEL: &str = "Move to Other View";
/// Default shortcuts, from the two `QKeySequence` constants described above.
pub const COPY_SHORTCUT: &str = "Shift+F5";
pub const MOVE_SHORTCUT: &str = "Shift+F6";
/// The two "What's This" sentences, whole (`0x231940`, `0x2319b8`).
pub const COPY_WHATS_THIS: &str =
    "This copies the selected items from the view in focus to the other view. \
     (Only available while in Split View mode.)";
pub const MOVE_WHATS_THIS: &str =
    "This moves the selected items from the view in focus to the other view. \
     (Only available while in Split View mode.)";

/// Which of the two things is happening. The refusal sentences differ only in
/// that word, which is how the original builds them too.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Copy,
    Move,
}

impl Mode {
    /// `Copy` / `Move`, capitalised as it starts a label.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Copy => "Copy",
            Mode::Move => "Move",
        }
    }

    /// The same word as it appears inside a sentence.
    pub fn lower(self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Move => "move",
        }
    }

    /// What the toast says once it is done.
    pub fn past(self) -> &'static str {
        match self {
            Mode::Copy => "Copied",
            Mode::Move => "Moved",
        }
    }

    pub fn other_label(self) -> &'static str {
        match self {
            Mode::Copy => COPY_LABEL,
            Mode::Move => MOVE_LABEL,
        }
    }

    pub fn other_shortcut(self) -> &'static str {
        match self {
            Mode::Copy => COPY_SHORTCUT,
            Mode::Move => MOVE_SHORTCUT,
        }
    }

    pub fn whats_this(self) -> &'static str {
        match self {
            Mode::Copy => COPY_WHATS_THIS,
            Mode::Move => MOVE_WHATS_THIS,
        }
    }

    /// The row the right-click menu carries — the addition, not the rebuild:
    /// the original's own *Copy To…* / *Move To…* come from KIO.
    pub fn chooser_label(self) -> &'static str {
        match self {
            Mode::Copy => "Copy To…",
            Mode::Move => "Move To…",
        }
    }
}

/// Why an action cannot run, in the original's own words; `None` when it can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    pub why: Option<String>,
}

impl Verdict {
    fn yes() -> Self {
        Verdict { ok: true, why: None }
    }

    fn no(why: impl Into<String>) -> Self {
        Verdict {
            ok: false,
            why: Some(why.into()),
        }
    }

    /// A refusal that has no sentence of its own in the original: the two rows
    /// are simply disabled while the window shows one view, and only the
    /// tooltip says why.
    fn quiet() -> Self {
        Verdict {
            ok: false,
            why: None,
        }
    }
}

/// Everything the four tests need, gathered by the caller so the rules can be
/// checked without a window.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// The window is showing two views (the original's byte at `+0x81`).
    pub split: bool,
    /// How many items are marked in the view in focus.
    pub selected: usize,
    /// One of the marked items already sits in the destination folder, i.e. its
    /// own parent is that folder — the original compares each item's URL, minus
    /// its file name, with the other view's URL.
    pub already_there: bool,
    /// The destination folder can be written into.
    pub dest_writable: bool,
    /// Every marked item can be taken out of where it is (for a move).
    pub may_move: bool,
}

/// The four refusals, in the order `updateActions` tests them.
///
/// | order | test | sentence |
/// | --- | --- | --- |
/// | 1 | not in split view | *(none — the row is disabled, the tooltip explains)* |
/// | 2 | nothing marked | `Cannot copy to other view: No files selected.` |
/// | 3 | destination already holds one | `… The other view already contains these items.` |
/// | 4 | destination not writable | `… You do not have permission to write into the destination folder.` |
/// | 5 | move, and the items cannot leave | `Cannot move to other view: You do not have permission to move items from this folder.` |
pub fn verdict(mode: Mode, facts: &Facts) -> Verdict {
    if !facts.split {
        return Verdict::quiet();
    }
    if facts.selected == 0 {
        return Verdict::no(format!(
            "Cannot {} to other view: No files selected.",
            mode.lower()
        ));
    }
    if facts.already_there {
        return Verdict::no(format!(
            "Cannot {} to other view: The other view already contains these items.",
            mode.lower()
        ));
    }
    if !facts.dest_writable {
        return Verdict::no(format!(
            "Cannot {} to other view: You do not have permission to write into the destination folder.",
            mode.lower()
        ));
    }
    if mode == Mode::Move && !facts.may_move {
        return Verdict::no(
            "Cannot move to other view: You do not have permission to move items from this folder."
                .to_string(),
        );
    }
    Verdict::yes()
}

/// Can this folder be written into? `access(2)` answers the question the
/// original asks through `KFileItem::isWritable`, which is also an `access`
/// check on a local file.
pub fn writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    match std::ffi::CString::new(dir.as_os_str().to_string_lossy().as_bytes()) {
        Ok(path) => unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 },
        Err(_) => false,
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Is one of these items already inside that folder? The original asks the same
/// question of every marked item before it lets the action through.
pub fn already_there(dest: &Path, sources: &[PathBuf]) -> bool {
    sources.iter().any(|src| match src.parent() {
        Some(parent) => same_folder(parent, dest),
        None => false,
    })
}

/// Can every one of these items be moved out of where it is? `supportsMoving`
/// in the original means, for a local file, that its folder can be written to.
pub fn may_move(sources: &[PathBuf]) -> bool {
    sources.iter().all(|src| match src.parent() {
        Some(parent) => writable(parent),
        None => false,
    })
}

/// A destination that overwrites nothing: `photo.jpg` becomes `photo (1).jpg`,
/// then `photo (2).jpg`, the same suggestion the desktop's own rename dialog
/// makes. A name with no extension just gets the number: `Notes (1)`.
pub fn free_target(dest: &Path, src: &Path) -> PathBuf {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "item".to_string());
    let first = dest.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    for n in 1..10_000 {
        let candidate = dest.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dest.join(format!("{stem} (copy){ext}"))
}

/// One item on its way, and the name it will have there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// What a run of the action will do: the items that can go, and the ones that
/// cannot with the reason why (a folder cannot be copied inside itself, and the
/// destination folder cannot go into itself).
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub skipped: Vec<(PathBuf, String)>,
}

pub fn plan(dest: &Path, sources: &[PathBuf]) -> Plan {
    let mut plan = Plan::default();
    for src in sources {
        if same_folder(src, dest) {
            plan.skipped
                .push((src.clone(), "that is the folder it would go into".to_string()));
            continue;
        }
        if src.is_dir() && dest.starts_with(src) {
            plan.skipped.push((
                src.clone(),
                "a folder cannot be copied into itself".to_string(),
            ));
            continue;
        }
        let to = free_target(dest, src);
        plan.steps.push(Step {
            from: src.clone(),
            to,
        });
    }
    plan
}

/// How the run went: how many items arrived, and the first thing that went
/// wrong, in a sentence.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub done: usize,
    pub failed: usize,
    pub first_error: Option<String>,
}

impl Outcome {
    /// The line the toast shows. Short, and it names the folder by its own name,
    /// the way the status bar does.
    pub fn summary(&self, mode: Mode, dest: &Path) -> String {
        let folder = dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dest.display().to_string());
        let one = format!(
            "{} 1 item to {}",
            mode.past(),
            if folder.is_empty() {
                "that folder".to_string()
            } else {
                folder.clone()
            }
        );
        let mut text = if self.done == 1 {
            one
        } else {
            format!("{} {} items to {}", mode.past(), self.done, folder)
        };
        if self.failed > 0 {
            text.push_str(&format!(" — {} could not be done", self.failed));
            if let Some(why) = &self.first_error {
                text.push_str(&format!(": {why}"));
            }
        }
        text
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)?.flatten() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if from.is_symlink() {
        // A link whose target is gone cannot be copied as a file; keeping the
        // link itself is what a copy should do.
        match std::fs::read_link(from) {
            Ok(target) => std::os::unix::fs::symlink(target, to),
            Err(_) => std::fs::copy(from, to).map(|_| ()),
        }
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Do it. A move first tries to rename (instant, same filesystem) and falls
/// back to copy-then-remove when the two folders sit on different disks, which
/// is what `rename(2)` refuses with `EXDEV`.
pub fn run(plan: &Plan, mode: Mode) -> Outcome {
    let mut outcome = Outcome::default();
    for step in &plan.steps {
        let result = match mode {
            Mode::Copy => copy_tree(&step.from, &step.to),
            Mode::Move => match std::fs::rename(&step.from, &step.to) {
                Ok(()) => Ok(()),
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                    copy_tree(&step.from, &step.to).and_then(|_| {
                        if step.from.is_dir() {
                            std::fs::remove_dir_all(&step.from)
                        } else {
                            std::fs::remove_file(&step.from)
                        }
                    })
                }
                Err(e) => Err(e),
            },
        };
        match result {
            Ok(()) => outcome.done += 1,
            Err(e) => {
                outcome.failed += 1;
                if outcome.first_error.is_none() {
                    let name = step
                        .from
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    outcome.first_error = Some(format!("{name}: {e}"));
                }
            }
        }
    }
    outcome
}

/// The small window behind *Copy To…* and *Move To…*: a folder to go to, typed
/// or picked from the list, and one button that does the work.
#[derive(Clone, Debug)]
pub struct Chooser {
    pub mode: Mode,
    pub paths: Vec<PathBuf>,
    pub text: String,
    pub error: Option<String>,
    pub focus: bool,
    /// What the person typed into the box; the list below fills it.
    pub places: Vec<(String, PathBuf)>,
}

impl Chooser {
    pub fn new(mode: Mode, paths: Vec<PathBuf>, start: PathBuf) -> Self {
        let mut places: Vec<(String, PathBuf)> = Vec::new();
        let mut push = |label: &str, path: Option<PathBuf>| {
            if let Some(path) = path {
                if path.is_dir() && !places.iter().any(|(_, p)| *p == path) {
                    places.push((label.to_string(), path));
                }
            }
        };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        push("Home", home.clone());
        for name in ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"] {
            if let Some(home) = &home {
                push(name, Some(home.join(name)));
            }
        }
        push("This folder", Some(start.clone()));
        Chooser {
            mode,
            paths,
            text: start.display().to_string(),
            error: None,
            focus: true,
            places,
        }
    }

    /// The folder the box currently names, if it names one.
    pub fn target(&self) -> Option<PathBuf> {
        let text = self.text.trim();
        if text.is_empty() {
            return None;
        }
        let expanded = if text == "~" {
            std::env::var_os("HOME").map(PathBuf::from)
        } else if let Some(rest) = text.strip_prefix("~/") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))
        } else {
            Some(PathBuf::from(text))
        };
        expanded.filter(|p| p.is_dir())
    }

    /// Check the box before anything is touched, so a refusal is a sentence in
    /// the window rather than a half-done job.
    pub fn check(&self) -> Result<PathBuf, String> {
        let dest = self
            .target()
            .ok_or_else(|| "Type a folder that exists, or pick one from the list.".to_string())?;
        if self.paths.is_empty() {
            return Err("Mark the items to copy first.".to_string());
        }
        if !writable(&dest) {
            return Err(format!(
                "{} cannot be written into.",
                dest.display()
            ));
        }
        let plan = plan(&dest, &self.paths);
        if plan.steps.is_empty() {
            return Err(plan
                .skipped
                .first()
                .map(|(name, why)| {
                    format!(
                        "{}: {why}.",
                        name.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "There is nothing to copy.".to_string()));
        }
        Ok(dest)
    }
}
