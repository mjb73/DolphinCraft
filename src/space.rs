//! Added feature: *Manage Disk Space Usage*.
//!
//! The original program has no disk-usage view of its own: its free-space
//! button carries the tooltip `%1 free out of %2 (%3% used)\nPress to manage
//! disk space usage.` (string 0x237458 in `/usr/bin/dolphin`), its **Tools**
//! menu carries a `manage_disk_space` action labelled *Manage Disk Space Usage*
//! (0x22d931) with the `filelight` icon, and that action is a submenu
//! (`DiskSpaceUsageMenu::aboutToShow`, `FUN_001e8430` @0x1e8430) which only ever
//! launches Filelight — or offers to install it, string 0x2375b0.
//!
//! This module answers the same question in the window itself: it walks the
//! folder once, adds up every file below it, and reports the total per child
//! folder and file, largest first, plus the biggest files found anywhere below.
//! Nothing is launched and nothing needs installing.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One row of the usage list: a file or a folder directly inside the folder the
/// window is looking at. A folder's `size` is everything below it, and `items`
/// is how many files were counted there.
#[derive(Clone)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub items: usize,
    /// The icon the row is drawn with, decided once during the walk.
    pub kind: crate::fsutil::Kind,
}

pub struct Big {
    pub path: PathBuf,
    pub size: u64,
}

pub enum Msg {
    /// One child of the starting folder is finished, so the list can fill in
    /// while the rest of the folder is still being walked.
    Child(Row),
    Progress { dirs: usize, files: usize, bytes: u64 },
    Done {
        tree: HashMap<PathBuf, Vec<Row>>,
        biggest: Vec<Big>,
        dirs: usize,
        files: usize,
        total: u64,
        skipped: usize,
    },
}

/// How many of the largest files are remembered. The list is a "free up space"
/// shortlist, not an inventory, and keeping it small means a walk over a whole
/// home directory never holds more than a few dozen paths in memory.
pub const BIGGEST_KEPT: usize = 40;

/// A fixed-size list that keeps the largest items seen so far, biggest first.
pub struct Biggest {
    limit: usize,
    pub items: Vec<Big>,
}

impl Biggest {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, size: u64, path: PathBuf) {
        if self.limit == 0 {
            return;
        }
        if self.items.len() < self.limit {
            self.items.push(Big { path, size });
            self.items.sort_by(|a, b| b.size.cmp(&a.size));
            return;
        }
        let smallest = self.items.last().map(|b| b.size).unwrap_or(0);
        if size <= smallest {
            return;
        }
        self.items.pop();
        self.items.push(Big { path, size });
        self.items.sort_by(|a, b| b.size.cmp(&a.size));
    }
}

#[derive(Default, Clone, Copy)]
pub struct Totals {
    pub dirs: usize,
    pub files: usize,
    pub bytes: u64,
    /// Entries that could not be read at all (a folder the person may not open,
    /// a file that vanished mid-walk). Counted, never invented into the total.
    pub skipped: usize,
}

/// Walk one folder and add up everything below it.
///
/// The rules, each of which the sheet records:
/// * symbolic links are never followed — a link to `/` inside a home folder
///   would otherwise make the walk endless and the total meaningless; a link is
///   counted at its own directory-entry size, like a file;
/// * hidden files and folders are always counted, whatever the file view is set
///   to show: a usage figure that hid `.cache` would be wrong;
/// * an entry that cannot be read is counted in `skipped` and adds nothing.
pub fn scan(
    root: &Path,
    cancel: &AtomicBool,
    tree: &mut HashMap<PathBuf, Vec<Row>>,
    biggest: &mut Biggest,
    totals: &mut Totals,
    on_child: &mut dyn FnMut(Row),
    on_progress: &mut dyn FnMut(Totals),
) -> u64 {
    let mut since_tick = 0usize;
    // The starting folder itself is not one of the folders "below" it, so the
    // walk counts folders from the first level down; the window's summary then
    // needs no correction.
    walk(
        root,
        0,
        cancel,
        tree,
        biggest,
        totals,
        on_child,
        on_progress,
        &mut since_tick,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk(
    dir: &Path,
    depth: usize,
    cancel: &AtomicBool,
    tree: &mut HashMap<PathBuf, Vec<Row>>,
    biggest: &mut Biggest,
    totals: &mut Totals,
    on_child: &mut dyn FnMut(Row),
    on_progress: &mut dyn FnMut(Totals),
    since_tick: &mut usize,
) -> u64 {
    totals.dirs += 1;
    let mut rows: Vec<Row> = Vec::new();
    let mut total = 0u64;
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => {
            totals.skipped += 1;
            tree.insert(dir.to_path_buf(), rows);
            return 0;
        }
    };
    for entry in read.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let Ok(md) = std::fs::symlink_metadata(&path) else {
            totals.skipped += 1;
            continue;
        };
        if md.is_dir() {
            let size = walk(
                &path,
                depth + 1,
                cancel,
                tree,
                biggest,
                totals,
                on_child,
                on_progress,
                since_tick,
            );
            let items = tree
                .get(&path)
                .map(|rows| {
                    rows.iter()
                        .map(|r| if r.is_dir { r.items } else { 1 })
                        .sum::<usize>()
                })
                .unwrap_or(0);
            let row = Row {
                path: path.clone(),
                name,
                is_dir: true,
                size,
                items,
                kind: crate::fsutil::Kind::Folder,
            };
            total += size;
            if depth == 0 {
                on_child(row.clone());
            }
            rows.push(row);
        } else {
            let size = md.len();
            totals.files += 1;
            totals.bytes += size;
            total += size;
            biggest.push(size, path.clone());
            let row = Row {
                path: path.clone(),
                name,
                is_dir: false,
                size,
                items: 1,
                kind: crate::fsutil::kind_of(&path, false, md.permissions().mode()),
            };
            if depth == 0 {
                on_child(row.clone());
            }
            rows.push(row);
        }
        *since_tick += 1;
        if *since_tick >= 200 {
            *since_tick = 0;
            on_progress(*totals);
        }
    }
    // Largest first: the point of the window is to put what takes the room at
    // the top. Two equal sizes fall back to the name order the file view uses.
    rows.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| crate::fsutil::name_cmp(&a.name, &b.name))
    });
    tree.insert(dir.to_path_buf(), rows);
    total
}

/// What share of a whole one entry takes, in whole percent, rounded the same
/// way the free-space figure is (`fsutil::used_percent`: half **up**, then
/// truncate, so 18.5 % reads 19 %).
pub fn share_percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }
    let scaled = part as f64 * 100.0 / whole as f64;
    (scaled + 0.5).floor() as u64
}

/// The window's own state: the scan in flight, what has arrived, and where the
/// person has drilled down to.
pub struct Space {
    pub root: PathBuf,
    pub show: PathBuf,
    /// The folders walked into inside the window, so "up" can go back without
    /// rescanning anything.
    pub trail: Vec<PathBuf>,
    rx: Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub scanning: bool,
    pub totals: Totals,
    pub partial: Vec<Row>,
    pub tree: HashMap<PathBuf, Vec<Row>>,
    pub biggest: Vec<Big>,
    pub skipped: usize,
    pub started: Instant,
    pub took: Option<Duration>,
    /// The row the person last clicked, so its full path and its ways to open
    /// can be offered under the list.
    pub picked: Option<PathBuf>,
    /// Set when the person cancels; the window keeps whatever was summed so far
    /// and says so instead of showing a half total as the truth.
    pub cancelled: bool,
}

impl Space {
    pub fn start(root: PathBuf) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let r = root.clone();
        std::thread::spawn(move || {
            let mut tree: HashMap<PathBuf, Vec<Row>> = HashMap::new();
            let mut biggest = Biggest::new(BIGGEST_KEPT);
            let mut totals = Totals::default();
            {
                let tx_child = tx.clone();
                let tx_prog = tx.clone();
                let mut on_child = move |row: Row| {
                    let _ = tx_child.send(Msg::Child(row));
                };
                let mut on_progress = move |t: Totals| {
                    let _ = tx_prog.send(Msg::Progress {
                        dirs: t.dirs,
                        files: t.files,
                        bytes: t.bytes,
                    });
                };
                scan(
                    &r,
                    &flag,
                    &mut tree,
                    &mut biggest,
                    &mut totals,
                    &mut on_child,
                    &mut on_progress,
                );
            }
            let _ = tx.send(Msg::Done {
                tree,
                biggest: biggest.items,
                dirs: totals.dirs,
                files: totals.files,
                total: totals.bytes,
                skipped: totals.skipped,
            });
        });
        Self {
            root: root.clone(),
            show: root,
            trail: Vec::new(),
            rx,
            cancel,
            scanning: true,
            totals: Totals::default(),
            partial: Vec::new(),
            tree: HashMap::new(),
            biggest: Vec::new(),
            skipped: 0,
            started: Instant::now(),
            took: None,
            picked: None,
            cancelled: false,
        }
    }

    /// Take whatever the walk has sent since the last frame. Called once per
    /// frame while the window is open.
    pub fn poll(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Child(row)) => self.partial.push(row),
                Ok(Msg::Progress { dirs, files, bytes }) => {
                    self.totals.dirs = dirs;
                    self.totals.files = files;
                    self.totals.bytes = bytes;
                }
                Ok(Msg::Done {
                    tree,
                    biggest,
                    dirs,
                    files,
                    total,
                    skipped,
                }) => {
                    self.tree = tree;
                    self.biggest = biggest;
                    self.totals.dirs = dirs;
                    self.totals.files = files;
                    self.totals.bytes = total;
                    self.skipped = skipped;
                    self.scanning = false;
                    self.took = Some(self.started.elapsed());
                    // The finished list supersedes the rows that arrived one by
                    // one; both are the same rows, the finished one is sorted.
                    self.partial.clear();
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.scanning = false;
                    break;
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancelled = true;
    }

    /// The rows for the folder the window is looking at. While the walk is still
    /// running this is the part of the starting folder that has arrived.
    pub fn rows(&self) -> Vec<Row> {
        if let Some(rows) = self.tree.get(&self.show) {
            return rows.clone();
        }
        let mut rows = self.partial.clone();
        rows.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| crate::fsutil::name_cmp(&a.name, &b.name))
        });
        rows
    }

    /// Everything below the folder on show, as one figure.
    pub fn shown_total(&self) -> u64 {
        if self.show == self.root {
            self.totals.bytes
        } else {
            self.tree
                .get(&self.show)
                .map(|rows| rows.iter().map(|r| r.size).sum())
                .unwrap_or(0)
        }
    }

    /// Walk into a folder already summed up, without touching the disk again.
    pub fn descend(&mut self, path: PathBuf) {
        self.trail.push(self.show.clone());
        self.show = path;
    }

    pub fn up(&mut self) {
        if let Some(prev) = self.trail.pop() {
            self.show = prev;
        }
    }

    /// The biggest files below the folder on show, so drilling into a folder
    /// narrows the shortlist with it.
    pub fn biggest_here(&self) -> Vec<&Big> {
        let mut out: Vec<&Big> = self
            .biggest
            .iter()
            .filter(|b| b.path.starts_with(&self.show))
            .collect();
        out.sort_by(|a, b| b.size.cmp(&a.size));
        out
    }

    /// How the walk is described in the window: the same shape as the status
    /// bar's line (`status_counts`), with the time it took appended once it is
    /// finished.
    pub fn summary(&self) -> String {
        let line = crate::status_counts(self.totals.dirs, self.totals.files, self.totals.bytes);
        let mut out = line;
        if let Some(took) = self.took {
            out.push_str(&format!("  ·  walked in {:.1} s", took.as_secs_f32()));
        }
        if self.skipped > 0 {
            out.push_str(&format!(
                "  ·  {} item{} could not be read",
                self.skipped,
                if self.skipped == 1 { "" } else { "s" }
            ));
        }
        out
    }
}
