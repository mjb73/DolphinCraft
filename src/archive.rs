//! Added feature: *Compress* and *Extract*.
//!
//! The original program has no compress or extract code of its own: its
//! right-click *Compress* submenu and its *Extract* row are installed by Ark's
//! KFileItemAction plugin, and `/usr/bin/dolphin` itself carries no more than
//! the settings row `Browse compressed files as folders`. A search of its
//! strings for `compress`/`extract` finds nothing else, so this window is an
//! addition rather than a rebuild — the same position the disk-usage window
//! stands in.
//!
//! What it does, in the window itself:
//! * **Compress** turns the marked items (or the folder on show) into one ZIP
//!   next to them, with a name the person can change first;
//! * **Extract** unpacks a ZIP or a tar archive into a folder of its own, or
//!   into the folder on show.
//!
//! Both run the archive programs the desktop already ships (`zip`, `unzip`,
//! `tar`) in the background, so a large folder never freezes the window, and a
//! missing program is reported as a sentence rather than an error code.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use crate::fsutil;

/// What kind of archive a name says it is. A folder is never an archive, even
/// when its name ends in `.zip` — that is a folder someone called that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zip,
    Tar,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Zip => "ZIP",
            Kind::Tar => "TAR",
        }
    }
}

pub fn kind_of(path: &Path) -> Option<Kind> {
    if path.is_dir() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        Some(Kind::Zip)
    } else if name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
    {
        Some(Kind::Tar)
    } else {
        None
    }
}

pub fn is_archive(path: &Path) -> bool {
    kind_of(path).is_some()
}

/// The name an archive of these items should start with: a single folder keeps
/// its own name, a single file loses its extension, and anything else is
/// `Archive` — the name Ark's own compress plugin gives a multi-file archive
/// ("Default name of a newly-created multi-file archive", string in
/// `compressfileitemaction.so`).
pub fn default_name(paths: &[PathBuf], dir: &Path) -> String {
    let base = match paths {
        [one] => {
            let name = one
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if one.is_dir() {
                name
            } else {
                let stem = one
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or(name.clone());
                if stem.is_empty() {
                    name
                } else {
                    stem
                }
            }
        }
        _ => {
            let _ = dir;
            "Archive".to_string()
        }
    };
    let base = if base.trim().is_empty() {
        "Archive".to_string()
    } else {
        base
    };
    format!("{base}.zip")
}

/// A destination that does not overwrite anything: `photos.zip` becomes
/// `photos (1).zip`, then `photos (2).zip`. The same "already there" suffix the
/// file manager uses when copying something into a folder that has one.
pub fn unique_path(want: &Path) -> PathBuf {
    if !want.exists() {
        return want.to_path_buf();
    }
    let parent = want.parent().unwrap_or(Path::new("."));
    let name = want
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Archive".to_string());
    // Keep `photos.tar.gz` whole: only a single extension is treated as one.
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    for n in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} (copy){ext}"))
}

/// The folder an archive unpacks into: its own name without the extension.
/// `photos.zip` → `photos`.
pub fn folder_name(archive: &Path) -> String {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Archive".to_string());
    let lower = name.to_lowercase();
    for suffix in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tbz2", ".txz", ".zip", ".tar",
    ] {
        if lower.ends_with(suffix) {
            let stem = &name[..name.len() - suffix.len()];
            if !stem.trim().is_empty() {
                return stem.to_string();
            }
        }
    }
    name
}

/// The name the archive gets, checked before anything is run. The rules are
/// the ones a file name has to keep: no slashes, nothing empty, and no leading
/// dash (the archive program would read that as a switch).
pub fn clean_name(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Give the archive a name first.".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("An archive name cannot contain a slash.".into());
    }
    if trimmed.starts_with('-') {
        return Err("An archive name cannot start with a dash.".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("That is not a name an archive can have.".into());
    }
    if trimmed.to_lowercase().ends_with(".zip") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}.zip"))
    }
}

/// `zip` is told to keep folders (`-r`), to stay quiet (`-q`) and to store a
/// symbolic link as a link instead of copying whatever it points at (`-y`), so
/// an archive of a home folder cannot quietly swallow another disk.
pub fn zip_args(dest_name: &str, names: &[String]) -> Vec<String> {
    let mut args = vec![
        "-r".to_string(),
        "-q".to_string(),
        "-y".to_string(),
        dest_name.to_string(),
    ];
    args.extend(names.iter().cloned());
    args
}

/// `unzip -q -o` unpacks quietly and overwrites, into a folder we name.
pub fn unzip_args(archive: &Path, into: &Path) -> Vec<String> {
    vec![
        "-q".to_string(),
        "-o".to_string(),
        archive.display().to_string(),
        "-d".to_string(),
        into.display().to_string(),
    ]
}

/// GNU tar reads the compression from the file itself when told `-xf`.
pub fn tar_args(archive: &Path, into: &Path) -> Vec<String> {
    vec![
        "-xf".to_string(),
        archive.display().to_string(),
        "-C".to_string(),
        into.display().to_string(),
    ]
}

/// Everything below these paths, the way the disk-usage window counts: links
/// are not followed, unreadable entries add nothing but do not fail the total.
pub fn total_size(paths: &[PathBuf]) -> u64 {
    fn walk(path: &Path, out: &mut u64) {
        let Ok(md) = std::fs::symlink_metadata(path) else {
            return;
        };
        if md.is_dir() {
            if let Ok(read) = std::fs::read_dir(path) {
                for entry in read.flatten() {
                    walk(&entry.path(), out);
                }
            }
        } else {
            *out += md.len();
        }
    }
    let mut out = 0;
    for p in paths {
        walk(p, &mut out);
    }
    out
}

/// One archive program running in the background. The window polls it once a
/// frame and shows a spinner until the answer arrives.
pub struct Job {
    rx: Receiver<Result<String, String>>,
    pub running: bool,
    pub program: String,
    pub args: Vec<String>,
}

impl Job {
    pub fn spawn(program: &str, args: Vec<String>, cwd: &Path) -> Job {
        let (tx, rx) = channel();
        let prog = program.to_string();
        let args_for_thread = args.clone();
        let cwd = cwd.to_path_buf();
        std::thread::spawn(move || {
            let out = Command::new(&prog)
                .args(&args_for_thread)
                .current_dir(&cwd)
                .stdin(Stdio::null())
                .output();
            let result = match out {
                Ok(o) if o.status.success() => Ok(String::new()),
                Ok(o) => {
                    let mut text = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    if text.is_empty() {
                        text = format!("{prog} stopped with {}", o.status);
                    }
                    Err(first_line(&text))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Err(format!("{prog} is not installed on this computer."))
                }
                Err(e) => Err(format!("{prog} could not be started: {e}")),
            };
            let _ = tx.send(result);
        });
        Job {
            rx,
            running: true,
            program: program.to_string(),
            args,
        }
    }

    /// `None` while it is still working, the answer once it is done.
    pub fn poll(&mut self) -> Option<Result<String, String>> {
        if !self.running {
            return None;
        }
        match self.rx.try_recv() {
            Ok(answer) => {
                self.running = false;
                Some(answer)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.running = false;
                Some(Err(format!("{} stopped without a word.", self.program)))
            }
        }
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(text)
        .to_string()
}

/// The *Compress* window's state.
pub struct Compress {
    pub paths: Vec<PathBuf>,
    /// The folder the archive is written into — the folder on show.
    pub dir: PathBuf,
    pub name: String,
    /// Whether the name field still waits for its first click.
    pub focus: bool,
    pub error: Option<String>,
    pub job: Option<Job>,
    pub last_dest: Option<PathBuf>,
    /// The size of everything marked, added up in the background when the
    /// window opens: walking a large folder on the frame that draws the window
    /// would freeze it for as long as the walk takes.
    total_rx: Receiver<u64>,
    pub total: Option<u64>,
}

impl Compress {
    pub fn new(paths: Vec<PathBuf>, dir: PathBuf) -> Self {
        let name = default_name(&paths, &dir);
        let (tx, total_rx) = channel();
        let counted = paths.clone();
        std::thread::spawn(move || {
            let _ = tx.send(total_size(&counted));
        });
        Self {
            paths,
            dir,
            name,
            focus: true,
            error: None,
            job: None,
            last_dest: None,
            total_rx,
            total: None,
        }
    }

    /// Take the size once the background walk has it.
    pub fn poll_total(&mut self) {
        if self.total.is_none() {
            if let Ok(size) = self.total_rx.try_recv() {
                self.total = Some(size);
            }
        }
    }

    pub fn running(&self) -> bool {
        self.job.as_ref().map(|j| j.running).unwrap_or(false)
    }

    /// Where the archive will land, avoiding anything already there.
    pub fn dest(&self) -> PathBuf {
        match clean_name(&self.name) {
            Ok(name) => unique_path(&self.dir.join(name)),
            Err(_) => self.dir.join("Archive.zip"),
        }
    }

    /// The names handed to `zip`, relative to the folder it runs in. A name
    /// that is not inside that folder is passed whole.
    pub fn names(&self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for p in &self.paths {
            let name = match p.strip_prefix(&self.dir) {
                Ok(rel) => rel.display().to_string(),
                Err(_) => p.display().to_string(),
            };
            if name.starts_with('-') {
                return Err("One of these items has a name starting with a dash, which the archive program would read as a switch.".into());
            }
            out.push(name);
        }
        Ok(out)
    }

    pub fn begin(&mut self) -> bool {
        if self.running() {
            return false;
        }
        self.error = None;
        let dest = match clean_name(&self.name) {
            Ok(name) => unique_path(&self.dir.join(&name)),
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        let names = match self.names() {
            Ok(n) => n,
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        if self.paths.is_empty() {
            self.error = Some("Nothing is marked to compress.".into());
            return false;
        }
        let dest_name = dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Archive.zip".to_string());
        let args = zip_args(&dest_name, &names);
        self.last_dest = Some(dest);
        self.job = Some(Job::spawn("zip", args, &self.dir));
        true
    }

    /// The sentence the window shows once the archive is written.
    pub fn done_message(&self) -> Option<String> {
        let dest = self.last_dest.clone()?;
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Some(format!("Created {} ({})", name, fsutil::format_size(size)))
    }

    pub fn poll(&mut self) -> Option<Result<String, String>> {
        let answer = self.job.as_mut().and_then(|j| j.poll());
        match &answer {
            Some(Err(e)) => self.error = Some(e.clone()),
            _ => {}
        }
        answer
    }
}

/// The *Extract* window's state.
pub struct Extract {
    pub archive: PathBuf,
    pub kind: Kind,
    /// Into a folder of its own (as the original's Ark does) or into the folder
    /// the archive sits in.
    pub own_folder: bool,
    pub error: Option<String>,
    pub job: Option<Job>,
    pub last_dest: Option<PathBuf>,
}

impl Extract {
    pub fn new(archive: PathBuf) -> Self {
        let kind = kind_of(&archive).unwrap_or(Kind::Zip);
        Self {
            archive,
            kind,
            own_folder: true,
            error: None,
            job: None,
            last_dest: None,
        }
    }

    pub fn running(&self) -> bool {
        self.job.as_ref().map(|j| j.running).unwrap_or(false)
    }

    /// Where it unpacks to: a new folder named after the archive, or the folder
    /// the archive is in.
    pub fn dest(&self) -> PathBuf {
        let parent = self
            .archive
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        if self.own_folder {
            unique_path(&parent.join(folder_name(&self.archive)))
        } else {
            parent
        }
    }

    pub fn begin(&mut self) -> bool {
        if self.running() {
            return false;
        }
        self.error = None;
        if !self.archive.is_file() {
            self.error = Some("That archive is not there any more.".into());
            return false;
        }
        if kind_of(&self.archive).is_none() {
            self.error = Some("This build unpacks ZIP and tar archives.".into());
            return false;
        }
        let dest = self.dest();
        if let Err(e) = std::fs::create_dir_all(&dest) {
            self.error = Some(format!("Could not make {}: {}", dest.display(), e));
            return false;
        }
        let (program, args) = match self.kind {
            Kind::Zip => ("unzip", unzip_args(&self.archive, &dest)),
            Kind::Tar => ("tar", tar_args(&self.archive, &dest)),
        };
        self.last_dest = Some(dest);
        let cwd = self
            .archive
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        self.job = Some(Job::spawn(program, args, &cwd));
        true
    }

    pub fn done_message(&self) -> Option<String> {
        let dest = self.last_dest.clone()?;
        let name = self.archive
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let folder = dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dest.display().to_string());
        if self.own_folder {
            Some(format!("Unpacked {name} into {folder}"))
        } else {
            Some(format!("Unpacked {name} into this folder"))
        }
    }

    pub fn poll(&mut self) -> Option<Result<String, String>> {
        let answer = self.job.as_mut().and_then(|j| j.poll());
        if let Some(Err(e)) = &answer {
            self.error = Some(e.clone());
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_destinations() {
        assert_eq!(clean_name(" photos ").unwrap(), "photos.zip");
        assert_eq!(clean_name("photos.ZIP").unwrap(), "photos.ZIP");
        assert!(clean_name("a/b").is_err());
        assert!(clean_name("-rf").is_err());
        assert_eq!(folder_name(Path::new("/x/photos.tar.gz")), "photos");
        assert_eq!(folder_name(Path::new("/x/a.zip")), "a");
    }
}
