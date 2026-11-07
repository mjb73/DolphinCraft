//! Per-tab, per-pane state: history, listing, selection, sorting, zoom.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::fsutil::{self, Entry};
use crate::search::{SearchFilters, SearchSession};
use crate::theme;

pub const TRASH_URI: &str = "trash:/";

/// Wheel delta that makes the original change the zoom by one step.
///
/// `DolphinView::wheelEvent` @0x272710 in libdolphinprivate.so.6 only acts when
/// the event carries `Qt::ControlModifier`, sums the incoming wheel delta in
/// the view object (this+0xcc) and calls `slotIncreaseZoom` / `slotDecreaseZoom`
/// once per ±0x78 = 120 units, subtracting those 120 from the sum and keeping
/// the rest for the next event. 120 units is one wheel notch in Qt (an eighth
/// of a degree per unit), so one notch is one zoom step. The step itself is
/// `setZoomLevel(level ± 1)` (`slotIncreaseZoom` @0x284b00), clamped to 0..=16
/// by `DolphinItemListView::setZoomLevel` @0x26dec0.
pub const WHEEL_UNITS_PER_STEP: f32 = 120.0;

/// Add one frame's worth of Ctrl+wheel delta (in Qt's 1/8-degree units) to the
/// running total and return how many zoom steps that buys, keeping the
/// leftover in `acc` exactly as the original does.
pub fn wheel_zoom_steps(acc: &mut f32, delta_units: f32) -> i32 {
    *acc += delta_units;
    let mut steps = 0;
    while *acc >= WHEEL_UNITS_PER_STEP {
        *acc -= WHEEL_UNITS_PER_STEP;
        steps += 1;
    }
    while *acc <= -WHEEL_UNITS_PER_STEP {
        *acc += WHEEL_UNITS_PER_STEP;
        steps -= 1;
    }
    steps
}

/// Turn one egui wheel event into Qt's wheel units. A line-based event (a real
/// mouse wheel, one line per notch) is 120 units; a pixel-based event (a
/// touchpad or a smooth-scrolling mouse) is converted with egui's own
/// 40 points per line, so a 40-pixel glide is one notch as well.
pub fn wheel_units(unit: egui::MouseWheelUnit, delta_y: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Line => delta_y * WHEEL_UNITS_PER_STEP,
        egui::MouseWheelUnit::Point => delta_y * (WHEEL_UNITS_PER_STEP / 40.0),
        egui::MouseWheelUnit::Page => delta_y * WHEEL_UNITS_PER_STEP * 3.0,
    }
}

/// Cut/copy buffer shared by every tab (as in the original, Ctrl+C / Ctrl+X
/// then Ctrl+V in any view).
#[derive(Clone, Default)]
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

impl Clipboard {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
    pub fn clear(&mut self) {
        self.paths.clear();
        self.cut = false;
    }
}

/// Where a typed or pasted address leads.
pub enum Location {
    Folder(PathBuf),
    File(PathBuf),
}

/// Turn whatever the person typed or pasted into a location. A pasted address
/// may be quoted, may arrive as a `file://` link, may start with `~`, and may
/// point at a file rather than a folder (the original opens the folder and
/// selects the file — see `Pane::apply_pending_select`).
pub fn resolve_location(raw: &str) -> Result<Location, String> {
    let text = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == ' ')
        .to_string();
    if text.is_empty() {
        return Err("Type or paste a location first.".to_string());
    }
    let text = match text.strip_prefix("file://") {
        Some(rest) => percent_decode(rest),
        None => text,
    };
    if text == "trash:" || text == "trash:/" || text.starts_with("trash:/") {
        return Ok(Location::Folder(PathBuf::from(TRASH_URI)));
    }
    let expanded = if text == "~" {
        fsutil::home_dir()
    } else if let Some(rest) = text.strip_prefix("~/") {
        fsutil::home_dir().join(rest)
    } else if text == "$HOME" {
        fsutil::home_dir()
    } else if let Some(rest) = text.strip_prefix("$HOME/") {
        fsutil::home_dir().join(rest)
    } else {
        PathBuf::from(&text)
    };
    if expanded.is_dir() {
        return Ok(Location::Folder(expanded));
    }
    if expanded.is_file() {
        return Ok(Location::File(expanded));
    }
    Err(format!("There is nothing at \u{201c}{text}\u{201d}."))
}

/// `%20` and friends, as they appear in a `file://` link copied from a browser.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Icons,
    Compact,
    Details,
}

impl ViewMode {
    pub fn index(self) -> usize {
        match self {
            ViewMode::Icons => 0,
            ViewMode::Compact => 1,
            ViewMode::Details => 2,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Icons => "Icons",
            ViewMode::Compact => "Compact",
            ViewMode::Details => "Details",
        }
    }
    pub fn all() -> [ViewMode; 3] {
        [ViewMode::Icons, ViewMode::Compact, ViewMode::Details]
    }
    pub fn from_index(i: usize) -> ViewMode {
        match i {
            0 => ViewMode::Icons,
            1 => ViewMode::Compact,
            _ => ViewMode::Details,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
    Modified,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Type => "Type",
            SortKey::Modified => "Modified",
        }
    }

    /// The role name the original files this key under. Every sortable column in
    /// `KFileItemModel` is a `QByteArray` role, and the Sort By menu stores that
    /// role in the action's data (`QAction::setData`, `createFileItemRolesActionGroup`
    /// @0x28d3f0 in libdolphinprivate.so.6); `Name` is the role `name`, the size
    /// column is `size`, the type column is `type`, and the date column is
    /// `modificationtime` — the first entry of the "sort descending" role table
    /// at 0x2f3960.
    pub fn role(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Type => "type",
            SortKey::Modified => "modificationtime",
        }
    }

    /// Position in the menu, which is also the index into the per-key direction
    /// memory (`Pane::preferred_desc`).
    pub fn index(self) -> usize {
        match self {
            SortKey::Name => 0,
            SortKey::Size => 1,
            SortKey::Type => 2,
            SortKey::Modified => 3,
        }
    }

    /// The inverse of `index`, used when the settings file is read back.
    pub fn from_index(i: usize) -> SortKey {
        match i {
            1 => SortKey::Size,
            2 => SortKey::Type,
            3 => SortKey::Modified,
            _ => SortKey::Name,
        }
    }

    /// Which way a key sorts the first time it is picked.
    ///
    /// `DolphinView::defaultSortOrderForRole` @0x273910 builds one static set of
    /// role names and answers "is this role in it". The set is built from the
    /// pointer table at 0x2f3960 (16 pointers, up to `PTR_FUN_002f39e0`) and holds
    /// exactly:
    ///
    /// modificationtime, creationtime, accesstime, deletiontime, imageDateTime,
    /// releaseYear, size, width, height, pageCount, wordCount, lineCount,
    /// rating, duration, bitrate
    ///
    /// (the sixteenth pointer lands inside the string "Color Space" and reads
    /// "ace", so it never matches a role). A member of that set starts
    /// **Descending**; every other role — `name`, `type`, `permissions`, `path`,
    /// `tags` … — starts **Ascending**. That is why picking Size or Modified in
    /// the original shows the biggest file or the newest date first, while Name
    /// and Type start at A and at the first type.
    pub fn descends_first(self) -> bool {
        match self {
            SortKey::Size | SortKey::Modified => true,
            SortKey::Name | SortKey::Type => false,
        }
    }
}

/// How two names are compared when sorting by name.
///
/// The original offers exactly three, as three radio buttons on the Sort page
/// of the View Properties dialog (`FUN_001e5750` @0x1e5750 in `/usr/bin/dolphin`).
/// Read out of that function's three `QRadioButton` constructions, in the order
/// the rows are laid out:
///
/// | row | text | string address |
/// | --- | --- | --- |
/// | 1 | `Natural` | 0x12eaf2 |
/// | 2 | `Alphabetical, case insensitive` | 0x1372e0 |
/// | 3 | `Alphabetical, case sensitive` | 0x12eafa |
///
/// `Natural` is the one the rebuild had hard-wired; the other two were missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameSort {
    Natural,
    CaseInsensitive,
    CaseSensitive,
}

impl NameSort {
    /// The exact label the original prints, used both in the menu and in the
    /// "Sort by" row of the app menu.
    pub fn label(self) -> &'static str {
        match self {
            NameSort::Natural => "Natural",
            NameSort::CaseInsensitive => "Alphabetical, case insensitive",
            NameSort::CaseSensitive => "Alphabetical, case sensitive",
        }
    }

    pub fn index(self) -> usize {
        match self {
            NameSort::Natural => 0,
            NameSort::CaseInsensitive => 1,
            NameSort::CaseSensitive => 2,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            1 => NameSort::CaseInsensitive,
            2 => NameSort::CaseSensitive,
            _ => NameSort::Natural,
        }
    }

    /// Compare two names the way the chosen mode does. The natural mode is the
    /// one `fsutil::name_cmp` already implements (digit runs compared by value);
    /// the two alphabetical modes are plain code-unit order, differing only in
    /// whether case is folded first. A case-insensitive tie falls back to the
    /// exact strings so the order stays total and stable.
    pub fn compare(self, a: &str, b: &str) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match self {
            NameSort::Natural => crate::fsutil::name_cmp(a, b),
            NameSort::CaseSensitive => a.cmp(b),
            NameSort::CaseInsensitive => {
                let (la, lb) = (a.to_lowercase(), b.to_lowercase());
                la.cmp(&lb).then_with(|| a.cmp(b))
            }
        }
    }
}

pub struct RenameState {
    pub index: usize,
    pub text: String,
    pub focus: bool,
}

pub enum Request {
    Navigate(PathBuf),
    NavigateOther(PathBuf),
    OpenInNewTab(PathBuf),
    OpenInSplit(PathBuf),
    AddBookmark(String, PathBuf),
    RemoveBookmark(PathBuf),
    Properties(Vec<PathBuf>),
    Trash(Vec<PathBuf>),
    Restore(Vec<PathBuf>),
    DeletePermanent(Vec<PathBuf>),
    /// Added: empty the Trash, the action the original's `Empty Trash` row runs.
    EmptyTrash,
    /// Added: put every remembered choice back to its factory default and forget
    /// the tabs of the last run. The original has no such row — it expects the
    /// person to walk its settings pages — but a single row is what "start over"
    /// means in a window that keeps everything by itself.
    ForgetSettings,
    BulkRename(Vec<PathBuf>),
    QuickLook(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf, String),
    OpenTerminal(PathBuf),
    CopyToOther(bool),
    Duplicate(Vec<PathBuf>),
    FolderSize(Vec<PathBuf>),
    /// Added feature: give folders a colour of the person's choosing (`None`
    /// clears it back to the normal one).
    FolderColor(Vec<PathBuf>, Option<[u8; 3]>),
    /// Added feature: show this picture in the full-window viewer adopted from
    /// the image browser. The flag starts its slideshow at once.
    OpenViewer(PathBuf, bool),
    /// Added feature: the gallery's thumbnail size or caption switches changed,
    /// so the choice is worth remembering.
    GalleryChanged,
    /// Added feature: open these files in a named program (the id of its
    /// `.desktop` entry). `remember` puts the choice into the desktop's own
    /// `mimeapps.list` — that is what the original's "Remember application
    /// association" box does.
    OpenWith {
        id: String,
        paths: Vec<PathBuf>,
        remember: Option<String>,
    },
    /// Added feature: the chooser behind "Other Application…".
    OpenWithChoose(Vec<PathBuf>),
    /// Added feature: *Manage Disk Space Usage* — add up everything below this
    /// folder and show what takes the room.
    SpaceUsage(PathBuf),
    /// Added feature: *Compress* — turn these items into one ZIP in that folder.
    Compress(Vec<PathBuf>, PathBuf),
    /// Added feature: *Extract* — unpack this archive next to itself.
    Extract(PathBuf),
    /// Added feature: show a file in its folder (the space window's shortlist,
    /// and the same thing the navigation panel's recent rows do).
    Reveal(PathBuf),
    /// Added feature: bring back the tab that was closed, by its place in the
    /// list of closed tabs (0 is the newest).
    ReopenClosedTab(usize),
    OpenPath(PathBuf),
    Cut,
    Copy,
    Paste,
    BeginRename(usize),
    /// A Ctrl+wheel zoom changed the size inside a pane; remember it for the
    /// next start.
    ZoomChanged,
    Toast(String),
}

pub struct Pane {
    pub id: usize,
    pub path: PathBuf,
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,
    pub entries: Vec<Entry>,
    /// The folder's whole listing, sorted. `entries` is what is actually shown,
    /// which is this list narrowed by the find bar's text. Keeping both means
    /// the filter is reversible and a re-sort still sees every item.
    pub all: Vec<Entry>,
    pub error: Option<String>,
    pub sel: BTreeSet<usize>,
    pub anchor: usize,
    pub cursor: usize,
    pub sort: SortKey,
    /// Which of the three name comparisons the original offers is in use
    /// (`FUN_001e5750` @0x1e5750, rows "Natural" / "Alphabetical, case
    /// insensitive" / "Alphabetical, case sensitive").
    pub name_sort: NameSort,
    pub asc: bool,
    pub folders_first: bool,
    /// The direction the person last chose for each sort key, in menu order.
    /// `DolphinView` keeps the same thing per view (`preferredSortOrder`
    /// @0x2859a0 looks the role up in a hash at this+0x130); picking a key that
    /// has never been used asks `defaultSortOrderForRole` instead. It is not
    /// written to the settings file — the original keeps it for the session and
    /// saves only the folder's own sort role and order in its view properties.
    pub preferred_desc: [Option<bool>; 4],
    pub view: ViewMode,
    /// The previews-on ladder. The original keeps **two** sizes per view mode,
    /// `IconSize` and `PreviewSize`, and `DolphinItemListView::setZoomLevel`
    /// @0x26dec0 writes the new size into whichever is in force: `PreviewSize`
    /// while previews are shown, `IconSize` while they are not. `zoom` is the
    /// previews-on ladder (`PreviewSize`), `zoom_plain` the previews-off one
    /// (`IconSize`), both indexed by view mode in Icons, Compact, Details order.
    pub zoom: [u8; 3],
    /// The previews-off ladder (`IconSize`): factory levels 2, 0, 0 from the
    /// measured factory sizes 32, 16, 16 px. Toggling `previews` switches which
    /// ladder the view draws from, so the size you zoomed the pictures to is
    /// waiting when you switch them back on.
    pub zoom_plain: [u8; 3],
    /// Added feature: the picture gallery presentation, adopted from the
    /// person's image browser. It replaces the icon/detail/column layout for
    /// this folder without touching the view mode underneath, so switching it
    /// off puts the folder back exactly as it was.
    pub pictures: bool,
    /// Thumbnail edge in the gallery, 50..=400 px (browser's own slider range).
    pub thumb_px: f32,
    /// Show the file name under a gallery thumbnail.
    pub gallery_names: bool,
    /// Show the "1920×1080:AR 1.78:MP 2.07" caption under a thumbnail.
    pub gallery_dims: bool,
    /// Rounded corners on the gallery's thumbnails.
    pub gallery_round: bool,
    /// Added feature: the wall is looking inside the sub-folders too. The wall
    /// then shows the pictures of the whole tree below the folder — the shape
    /// of a photo library sorted into year or month folders — through the same
    /// listing a search uses, so marking, deleting and the viewer keep working
    /// on the real files. Off by default and not remembered: it belongs to the
    /// folder on screen, not to the window.
    pub gallery_below: bool,
    pub show_hidden: bool,
    pub previews: bool,
    pub filter: String,
    pub filter_on: bool,
    pub filter_focus: bool,
    /// Added: the find bar's scope. `false` = match names in this folder only
    /// (a live filter); `true` = search everything below it as well. The
    /// original spends a hidden keystroke (Enter) on that choice; here it is a
    /// visible pair of chips, so the two behaviours are never confused.
    pub find_deep: bool,
    pub location_edit: Option<String>,
    pub location_focus: bool,
    /// After a location was typed or pasted, the file inside the folder that
    /// should end up selected (the original selects the file once the listing
    /// has arrived — DolphinView::selectFileOnceAvailable, 0x173b40).
    pub pending_select: Option<String>,
    /// Select the whole address the next time the location field is drawn, so a
    /// paste replaces the current path instead of being appended to it.
    pub location_select: bool,
    pub rename: Option<RenameState>,
    pub search: Option<SearchSession>,
    pub scroll_to: Option<usize>,
    pub last_click: Option<(usize, std::time::Instant)>,
    pub loading: bool,
    pub selected_size: Option<u64>,
    /// Ctrl+wheel delta gathered so far, in Qt's 1/8-degree wheel units, kept
    /// between frames so a slow glide still reaches a full step.
    pub wheel_acc: f32,
}

impl Pane {
    pub fn new(id: usize, path: PathBuf) -> Self {
        let mut pane = Self {
            id,
            path: PathBuf::new(),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            all: Vec::new(),
            error: None,
            sel: BTreeSet::new(),
            anchor: 0,
            cursor: 0,
            sort: SortKey::Name,
            name_sort: NameSort::Natural,
            asc: true,
            folders_first: true,
            preferred_desc: [None; 4],
            view: ViewMode::Icons,
            zoom: theme::default_zoom(true),
            zoom_plain: theme::default_zoom(false),
            pictures: false,
            thumb_px: crate::gallery::THUMB_DEFAULT,
            gallery_names: true,
            gallery_dims: false,
            gallery_round: true,
            gallery_below: false,
            show_hidden: false,
            previews: true,
            filter: String::new(),
            filter_on: false,
            filter_focus: false,
            find_deep: false,
            location_edit: None,
            location_focus: false,
            pending_select: None,
            location_select: false,
            rename: None,
            search: None,
            scroll_to: None,
            last_click: None,
            loading: false,
            selected_size: None,
            wheel_acc: 0.0,
        };
        pane.load(path);
        pane
    }

    pub fn is_trash(&self) -> bool {
        self.path.as_os_str() == TRASH_URI
    }

    pub fn display_path(&self) -> String {
        if self.is_trash() {
            "Trash".to_string()
        } else {
            self.path.display().to_string()
        }
    }

    pub fn breadcrumbs(&self) -> Vec<(String, PathBuf)> {
        if self.is_trash() {
            return vec![("Trash".to_string(), PathBuf::from(TRASH_URI))];
        }
        let mut out = Vec::new();
        let mut acc = PathBuf::from("/");
        out.push(("root".to_string(), acc.clone()));
        for comp in self.path.components() {
            use std::path::Component;
            if let Component::Normal(part) = comp {
                acc = acc.join(part);
                out.push((part.to_string_lossy().to_string(), acc.clone()));
            }
        }
        if out.len() > 1 {
            out.remove(0);
            out.insert(0, ("/".to_string(), PathBuf::from("/")));
        }
        out
    }

    pub fn load(&mut self, path: PathBuf) {
        // Show the folder itself, not the wording that led to it: a path with
        // `..` in it would otherwise leave a segment in the location bar that
        // is not a folder.
        self.path = crate::fsutil::tidy_path(&path);
        self.rename = None;
        self.location_edit = None;
        self.search = None;
        // A wall that was looking below one folder does not follow into the
        // next one: the switch belongs to what is on screen.
        self.gallery_below = false;
        self.error = None;
        self.reload();
    }

    /// Added feature: turn the wall's "Sub-folders" switch on or off. On starts
    /// a walk of the folder and everything under it, keeping pictures only; off
    /// throws the walk away and shows the folder itself again, exactly as it
    /// was. The walk is the same one the find bar uses, so the result arrives in
    /// batches and a big tree never freezes the window.
    pub fn set_pictures_below(&mut self, on: bool) {
        if on == self.gallery_below {
            return;
        }
        self.gallery_below = on;
        self.sel.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.selected_size = None;
        if on {
            let filters = crate::search::SearchFilters {
                pictures_only: true,
                ..Default::default()
            };
            let mut session = crate::search::SearchSession::start(
                self.path.clone(),
                String::new(),
                filters,
            );
            session.pictures_below = true;
            self.search = Some(session);
        } else {
            self.search = None;
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        self.sel.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.selected_size = None;
        self.loading = false;
        if self.is_trash() {
            self.all = fsutil::list_trash();
            self.error = None;
        } else {
            match fsutil::read_folder(&self.path, self.show_hidden) {
                Ok(mut list) => {
                    list.retain(|e| self.show_hidden || !e.hidden);
                    self.all = list;
                    self.error = None;
                }
                Err(e) => {
                    self.entries.clear();
                    self.all.clear();
                    self.error = Some(e);
                }
            }
        }
        self.sort_entries();
        self.apply_pending_select();
    }

    /// The original view selects a file as soon as it appears in the listing
    /// (DolphinView::selectFileOnceAvailable @0x173b40), which is how a typed or
    /// pasted address for a *file* lands on that file with its folder open.
    pub fn apply_pending_select(&mut self) {
        let Some(name) = self.pending_select.take() else {
            return;
        };
        if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
            self.sel.clear();
            self.sel.insert(idx);
            self.anchor = idx;
            self.cursor = idx;
            self.scroll_to = Some(idx);
        }
    }

    /// Navigate to something the person typed or pasted into the location bar.
    /// A folder opens; a file opens its folder with the file selected. Returns
    /// the message to show when the address leads nowhere.
    pub fn navigate_input(&mut self, raw: &str) -> Option<String> {
        match resolve_location(raw) {
            Ok(Location::Folder(path)) => {
                self.pending_select = None;
                self.navigate(path);
                None
            }
            Ok(Location::File(path)) => {
                let parent = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(fsutil::home_dir);
                self.pending_select = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string());
                self.navigate(parent);
                None
            }
            Err(message) => Some(message),
        }
    }

    pub fn navigate(&mut self, path: PathBuf) {
        if path == self.path {
            self.reload();
            return;
        }
        self.back.push(self.path.clone());
        if self.back.len() > 100 {
            self.back.remove(0);
        }
        self.forward.clear();
        self.load(path);
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.back.pop() {
            self.forward.push(self.path.clone());
            self.load(prev);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            self.back.push(self.path.clone());
            self.load(next);
        }
    }

    pub fn go_up(&mut self) {
        if self.is_trash() {
            self.navigate(fsutil::home_dir());
            return;
        }
        if let Some(parent) = self.path.parent() {
            let p = parent.to_path_buf();
            self.navigate(p);
        }
    }

    /// Pick a sort key from the Sort By menu.
    ///
    /// `DolphinViewActionHandler::slotSortTriggered` @0x291f30 only records which
    /// action is ticked and calls `DolphinView::setSortRole`; it never reverses the
    /// order, so choosing the key that is already chosen does nothing. The
    /// direction is its own pair of menu items (`ascending` / `descending`, see
    /// `set_sort_dir`). `DolphinView::setSortRole` @0x2716d0 then asks
    /// `preferredSortOrder` @0x2859a0 for the order to use with the new role: the
    /// order last chosen for that role, or `defaultSortOrderForRole` @0x273910
    /// when it has never been chosen.
    ///
    /// Returns true when the listing had to be re-sorted.
    pub fn choose_sort(&mut self, key: SortKey) -> bool {
        if self.sort == key {
            return false;
        }
        self.sort = key;
        let desc = self
            .preferred_desc
            .get(key.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| key.descends_first());
        self.asc = !desc;
        self.sort_entries();
        true
    }

    /// Choose Ascending or Descending from the pair under the key list.
    ///
    /// `DolphinViewActionHandler::slotSortOrderChanged` @0x292640 ticks the two
    /// actions and calls `DolphinView::setPreferredSortOrder(role, order)`, which
    /// remembers the order for the *current role only* — so a direction chosen for
    /// Size stays with Size and does not follow the person to Name.
    pub fn set_sort_dir(&mut self, descending: bool) -> bool {
        if let Some(slot) = self.preferred_desc.get_mut(self.sort.index()) {
            *slot = Some(descending);
        }
        if self.asc == descending {
            self.asc = !descending;
            self.sort_entries();
            return true;
        }
        false
    }

    pub fn sort_entries(&mut self) {
        let sort = self.sort;
        let name_sort = self.name_sort;
        let asc = self.asc;
        let folders_first = self.folders_first;
        self.all.sort_by(|a, b| {
            if folders_first && a.is_dir != b.is_dir {
                return if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            let ord = match sort {
                SortKey::Name => name_sort.compare(&a.name, &b.name),
                SortKey::Size => a
                    .size
                    .cmp(&b.size)
                    .then_with(|| name_sort.compare(&a.name, &b.name)),
                SortKey::Type => a
                    .type_label()
                    .cmp(&b.type_label())
                    .then_with(|| name_sort.compare(&a.name, &b.name)),
                SortKey::Modified => a
                    .modified
                    .cmp(&b.modified)
                    .then_with(|| name_sort.compare(&a.name, &b.name)),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
        self.refilter();
    }

    /// Narrow the shown listing to the find bar's text. Called whenever the text,
    /// the filter's on/off state or the folder changes, so what is drawn and what
    /// the selection indices point at stay the same list.
    pub fn refilter(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        if !self.filter_on || needle.is_empty() {
            self.entries = self.all.clone();
            return;
        }
        self.entries = self
            .all
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();
    }

    pub fn visible(&self) -> Vec<usize> {
        if !self.filter_on || self.filter.trim().is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.sel
            .iter()
            .filter_map(|i| self.entries.get(*i).map(|e| e.path.clone()))
            .collect()
    }

    pub fn primary_selection(&self) -> Option<&Entry> {
        self.sel
            .iter()
            .next()
            .and_then(|i| self.entries.get(*i))
            .or_else(|| self.entries.get(self.cursor))
    }

    pub fn select_single(&mut self, idx: usize) {
        self.sel.clear();
        self.sel.insert(idx);
        self.anchor = idx;
        self.cursor = idx;
    }

    pub fn toggle_select(&mut self, idx: usize) {
        if !self.sel.remove(&idx) {
            self.sel.insert(idx);
        }
        self.anchor = idx;
        self.cursor = idx;
    }

    /// The indices the person can see and pick right now. In the list and grid
    /// views that is the whole filtered listing; on the picture wall it is the
    /// pictures alone, because the wall shows nothing else. Ctrl+A, Shift+click
    /// and the arrow keys all step through this list, so a document or a folder
    /// that the wall does not draw can never be marked — and therefore never
    /// deleted — from a view the person cannot see it in.
    pub fn shown_indices(&self) -> Vec<usize> {
        let mut shown = self.visible();
        if self.pictures {
            shown.retain(|i| {
                self.entries
                    .get(*i)
                    .map(|e| !e.is_dir && e.is_image())
                    .unwrap_or(false)
            });
        }
        shown
    }

    /// Turn the picture wall on or off. Anything marked that the wall does not
    /// show is unmarked at the same moment, so switching to the wall and pressing
    /// Delete cannot remove a file that was never on screen.
    pub fn set_pictures(&mut self, on: bool) {
        self.pictures = on;
        if !on {
            return;
        }
        let shown = self.shown_indices();
        self.sel.retain(|i| shown.contains(i));
        if !shown.contains(&self.cursor) {
            if let Some(&first) = shown.first() {
                self.cursor = first;
            }
        }
    }

    pub fn select_range(&mut self, idx: usize) {
        let shown = self.shown_indices();
        let Some(b) = shown.iter().position(|i| *i == idx) else {
            // Only reachable if the wall changed under the pointer; marking the
            // one item keeps the click honest.
            self.select_single(idx);
            return;
        };
        let a = shown
            .iter()
            .position(|i| *i == self.anchor)
            .unwrap_or(b);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        self.sel.clear();
        for i in &shown[lo..=hi] {
            self.sel.insert(*i);
        }
        self.cursor = idx;
    }

    pub fn select_all(&mut self) {
        self.sel = self.shown_indices().into_iter().collect();
    }

    pub fn icon_size(&self) -> f32 {
        theme::icon_size_for_zoom(self.zoom_level())
    }

    /// The zoom level currently in force. The original derives it every time
    /// from the size that is in force — `DolphinView::updateDefaultZoomLevel`
    /// @0x27dd60 and `DolphinItemListView::updateGridSize` @0x26d940 both read
    /// `PreviewSize` while previews are shown and `IconSize` while they are off
    /// — so switching previews switches the size the view draws at.
    pub fn zoom_level(&self) -> u8 {
        let i = self.view.index();
        if self.previews {
            self.zoom[i]
        } else {
            self.zoom_plain[i]
        }
    }

    /// Set the ladder that is in force, the way `setZoomLevel` @0x26dec0 does:
    /// clamps to the ladder, then writes the one size the mode is using.
    pub fn set_zoom_level(&mut self, level: u8) {
        let i = self.view.index();
        let level = level.min(theme::MAX_ZOOM);
        if self.previews {
            self.zoom[i] = level;
        } else {
            self.zoom_plain[i] = level;
        }
    }

    pub fn zoom_by(&mut self, delta: i32) {
        let z = self.zoom_level() as i32 + delta;
        // Extended past the original's 16-level ceiling so the icons view can
        // reach 400 px thumbnails (`theme::MAX_ZOOM`).
        self.set_zoom_level(z.clamp(0, theme::MAX_ZOOM as i32) as u8);
    }

    pub fn start_search(&mut self) {
        let query = self.filter.clone();
        let filters = SearchFilters::default();
        self.search = Some(SearchSession::start(
            if self.is_trash() {
                fsutil::home_dir()
            } else {
                self.path.clone()
            },
            query,
            filters,
        ));
        self.sel.clear();
        self.cursor = 0;
    }

    /// Added: choose where the find bar looks, keeping the results in step with
    /// the text already typed. "This folder" filters the listing live; "In
    /// subfolders" starts the background search.
    pub fn set_find_scope(&mut self, deep: bool) {
        if self.find_deep == deep {
            return;
        }
        self.find_deep = deep;
        if deep {
            if !self.filter.trim().is_empty() {
                self.start_search();
            }
        } else {
            self.search = None;
            self.sel.clear();
            self.cursor = 0;
        }
    }

    /// Entries that are shown: search results when searching, else the folder.
    pub fn display_entries(&self) -> &[Entry] {
        match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        }
    }

    pub fn open_entry(&mut self, idx: usize, reqs: &mut Vec<Request>) {
        let Some(entry) = self.display_entries().get(idx).cloned() else {
            return;
        };
        if entry.is_dir {
            let target = if self.is_trash() {
                if entry.path.starts_with(fsutil::trash_root().join("files")) {
                    // Stay in the trash when a folder there is opened.
                    entry.path.clone()
                } else {
                    entry.path.clone()
                }
            } else {
                entry.path.clone()
            };
            self.navigate(target);
        } else {
            reqs.push(Request::OpenPath(entry.path.clone()));
        }
    }

    pub fn entry_at(&self, idx: usize) -> Option<&Entry> {
        self.display_entries().get(idx)
    }

    pub fn current_dir(&self) -> PathBuf {
        if self.is_trash() {
            fsutil::home_dir()
        } else {
            self.path.clone()
        }
    }

    pub fn path_is_dir(&self) -> bool {
        Path::new(&self.path).is_dir()
    }

    // -----------------------------------------------------------------------
    // Added feature: the picture gallery
    // -----------------------------------------------------------------------

    /// The pictures the gallery and its viewer work on, in the order the pane
    /// shows them: the visible listing (search results while searching), with
    /// folders and non-picture files dropped. The image browser pages through
    /// exactly this list, so arrow keys step through pictures, not files.
    pub fn gallery_images(&self) -> Vec<PathBuf> {
        self.display_entries()
            .iter()
            .filter(|e| !e.is_dir && e.is_image())
            .map(|e| e.path.clone())
            .collect()
    }

    /// Whether the gallery has anything to lay out: a folder of only documents
    /// still shows its files, as the browser shows HTML files beside pictures.
    pub fn pictures_shown(&self) -> bool {
        self.pictures
    }

    /// Step the gallery's thumbnail size by one slider notch, staying inside
    /// the browser's 50..400 px range.
    pub fn thumb_by(&mut self, delta: f32) {
        self.thumb_px = crate::gallery::clamp_thumb(self.thumb_px + delta);
    }
}
