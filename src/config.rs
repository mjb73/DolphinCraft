//! Small JSON config: bookmarks and the last window state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The version written by this build; see `Config::settings_version`.
pub const SETTINGS_VERSION: u32 = 1;

/// One row of the person's own part of the Shortcuts panel.
///
/// Added feature: a **group** is the same row with `group` set, its own name and
/// no folder — a container the person drops shortcuts into, folded away when
/// they are not wanted. A plain shortcut carries a folder and no children, so
/// the older shape (just `name` and `path`) still loads: every field added here
/// defaults to the plain-shortcut case.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub name: String,
    #[serde(default)]
    pub path: PathBuf,
    /// This row holds other shortcuts instead of standing for a folder.
    #[serde(default)]
    pub group: bool,
    /// A group's disclosure state, remembered so a folded group comes back folded.
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub children: Vec<Bookmark>,
}

impl Bookmark {
    pub fn plain(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            path,
            group: false,
            open: false,
            children: Vec::new(),
        }
    }

    pub fn group(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::new(),
            group: true,
            open: true,
            children: Vec::new(),
        }
    }
}

/// Where one of the person's shortcuts sits: a row at the top of the list, or a
/// row inside a group.
///
/// Added feature (shortcut groups): the panel used to address its rows by their
/// index among the top-level shortcuts, which cannot name a row inside a group.
/// An index equal to the length of the list is the position *after* the last
/// row, which is where a drop at the end lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Top(usize),
    In(usize, usize),
}

impl Spot {
    pub fn row(self) -> usize {
        match self {
            Spot::Top(i) | Spot::In(_, i) => i,
        }
    }

    pub fn group(self) -> Option<usize> {
        match self {
            Spot::Top(_) => None,
            Spot::In(g, _) => Some(g),
        }
    }
}

/// The row a spot names, if it is a row and not the gap after the last one.
pub fn row_at(tree: &[Bookmark], spot: Spot) -> Option<&Bookmark> {
    match spot {
        Spot::Top(i) => tree.get(i),
        Spot::In(g, i) => tree.get(g)?.children.get(i),
    }
}

pub fn row_at_mut(tree: &mut [Bookmark], spot: Spot) -> Option<&mut Bookmark> {
    match spot {
        Spot::Top(i) => tree.get_mut(i),
        Spot::In(g, i) => tree.get_mut(g)?.children.get_mut(i),
    }
}

/// How many rows sit in the list a spot points into: the top level, or one
/// group's children.
pub fn rows_in(tree: &[Bookmark], spot: Spot) -> usize {
    match spot {
        Spot::Top(_) => tree.len(),
        Spot::In(g, _) => tree.get(g).map(|b| b.children.len()).unwrap_or(0),
    }
}

/// Where a folder's shortcut lives, searching groups as well.
pub fn spot_of_path(tree: &[Bookmark], path: &Path) -> Option<Spot> {
    for (i, bm) in tree.iter().enumerate() {
        if !bm.group && bm.path == path {
            return Some(Spot::Top(i));
        }
        for (j, child) in bm.children.iter().enumerate() {
            if !child.group && child.path == path {
                return Some(Spot::In(i, j));
            }
        }
    }
    None
}

/// Every folder the shortcuts point at, groups and their children included.
pub fn shortcut_paths(tree: &[Bookmark]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for bm in tree {
        if !bm.group && !bm.path.as_os_str().is_empty() {
            out.push(bm.path.clone());
        }
        for child in &bm.children {
            if !child.group && !child.path.as_os_str().is_empty() {
                out.push(child.path.clone());
            }
        }
    }
    out
}

/// The row in `spot`, taken out of the tree.
fn take_row(tree: &mut Vec<Bookmark>, spot: Spot) -> Option<Bookmark> {
    match spot {
        Spot::Top(i) => {
            if i < tree.len() {
                Some(tree.remove(i))
            } else {
                None
            }
        }
        Spot::In(g, i) => {
            let grp = tree.get_mut(g).filter(|b| b.group)?;
            if i < grp.children.len() {
                Some(grp.children.remove(i))
            } else {
                None
            }
        }
    }
}

/// Move the row at `from` so it lands in the gap before `to`.
///
/// `to` may be one past the end of its list, which is the drop at the bottom.
/// A group can be reordered among the top-level rows but never moved inside
/// another group — one level of nesting is what the panel draws.
pub fn move_row(tree: &mut Vec<Bookmark>, from: Spot, to: Spot) -> bool {
    let Some(item) = row_at(tree, from) else {
        return false;
    };
    let is_group = item.group;
    // The target has to be a real list: the top level, or a group.
    let dst_ok = match to {
        Spot::Top(t) => t <= tree.len(),
        Spot::In(g, t) => tree.get(g).map(|b| b.group && t <= b.children.len()).unwrap_or(false),
    };
    if !dst_ok || (is_group && matches!(to, Spot::In(..))) {
        return false;
    }
    // A drop on the row's own place, or on the gap right below it, is a no-op.
    if from == to {
        return false;
    }
    if from.group() == to.group() && to.row() == from.row() + 1 {
        return false;
    }
    let Some(item) = take_row(tree, from) else {
        return false;
    };
    // Taking the row out shifts anything below it in the same list.
    let dst = match to {
        Spot::Top(t) => {
            let t = if matches!(from, Spot::Top(f) if f < t) {
                t - 1
            } else {
                t
            };
            (None, t)
        }
        Spot::In(g, t) => {
            // A group index counts top-level rows, so a row leaving above the
            // group moves the group up by one.
            let g = if matches!(from, Spot::Top(f) if f < g) {
                g - 1
            } else {
                g
            };
            let t = if matches!(from, Spot::In(fg, fi) if fg == g && fi < t) {
                t - 1
            } else {
                t
            };
            (Some(g), t)
        }
    };
    match dst.0 {
        None => {
            let at = dst.1.min(tree.len());
            tree.insert(at, item);
        }
        Some(g) => {
            let Some(grp) = tree.get_mut(g) else {
                return false;
            };
            let at = dst.1.min(grp.children.len());
            grp.children.insert(at, item);
        }
    }
    if let Some(grp) = dst.0.and_then(|g| tree.get_mut(g)) {
        // A group that just took a row opens, so the row is not dropped out of
        // sight; the person can fold it again with one click.
        grp.open = true;
    }
    true
}

/// Take one shortcut out by the folder it stands for, wherever it sits.
pub fn remove_path(tree: &mut Vec<Bookmark>, path: &Path) -> bool {
    if let Some(Spot::In(g, i)) = spot_of_path(tree, path) {
        if let Some(grp) = tree.get_mut(g) {
            grp.children.remove(i);
            return true;
        }
    }
    match spot_of_path(tree, path) {
        Some(Spot::Top(i)) => {
            tree.remove(i);
            true
        }
        _ => false,
    }
}


/// Added feature: the colour the person gave one row of the Shortcuts panel.
/// The original has no place colours at all, so this is stored next to the
/// shortcuts in the rebuild's own config, keyed by the folder the row stands
/// for. Folder colours go into the folder's `.directory` file instead, because
/// there they can travel with the folder; a panel row has no such file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShortcutColor {
    pub path: PathBuf,
    pub rgb: [u8; 3],
}

/// One tab restored from the last run, by the folder it was showing.
///
/// The original saves the whole session — "Folders, tabs, and window state from
/// last time" is the wording of its Startup page (`FoldersTabsSettingsPage`,
/// string at 0x137dc8 in `/usr/bin/dolphin`) — in
/// `DolphinMainWindow::slotSaveSession`, so tabs, the active tab and the window
/// come back on the next start. This build stores just the folder of each tab's
/// active pane: the per-tab split state and the history are session details the
/// person has not asked to keep.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedTab {
    pub path: PathBuf,
    /// The sort this tab was showing. `None` in a file written before tabs kept
    /// their own sort, in which case the window's saved sort is used.
    #[serde(default)]
    pub sort_key: Option<usize>,
    #[serde(default)]
    pub sort_desc: Option<bool>,
    /// The view this tab was showing (0 icons, 1 compact, 2 details). `None` in a
    /// file written before a tab kept its own view.
    #[serde(default)]
    pub view: Option<usize>,
    /// Whether this tab had the picture view up.
    #[serde(default)]
    pub pictures: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Which set of factory defaults this file was written with. Files from
    /// before the slideshow's default moved to 1.5 s have no version (0); they
    /// are brought up to date once, in `migrate`.
    #[serde(default)]
    pub settings_version: u32,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub last_dir: String,
    #[serde(default)]
    pub split: bool,
    #[serde(default = "yes")]
    pub places_visible: bool,
    #[serde(default)]
    pub info_panel: bool,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "yes")]
    pub previews: bool,
    #[serde(default = "details")]
    pub view: usize,
    #[serde(default = "default_zoom")]
    pub zoom: [u8; 3],
    /// The previews-off ladder (`IconSize`): levels 2, 0, 0, from the measured
    /// factory sizes 32, 16, 16 px. Written whenever the person zooms with
    /// previews off, so the two sizes behave like the original's `IconSize`
    /// and `PreviewSize` rather than one shared number.
    #[serde(default = "default_zoom_plain")]
    pub zoom_plain: [u8; 3],
    #[serde(default = "yes")]
    pub folders_first: bool,
    /// Whether the navigation panel keeps the current folder's row in view
    /// (`[FoldersPanel] AutoScrolling`, default true — the `ItemBool` in
    /// `FoldersPanelSettings::FoldersPanelSettings` @0x1f9a00 in
    /// `/usr/bin/dolphin`, and the toggle `FUN_00217320` @0x217320 applies from
    /// the panel's own menu). With it on, walking into a folder elsewhere
    /// scrolls the panel to the row that folder owns instead of leaving the row
    /// off the top or bottom edge.
    #[serde(default = "yes")]
    pub places_autoscroll: bool,
    /// `[InformationPanel] dateFormat`, index into the original's two choices:
    /// 0 `LongFormat` (the factory default) and 1 `ShortFormat`. The
    /// `ItemEnum` is built in `InformationPanelSettings::InformationPanelSettings`
    /// @0x1fe410 in `/usr/bin/dolphin`; the two names sit at 0x22cfa4 and
    /// 0x22cfba. Relative wording stays on either way, because
    /// `[ContentDisplay] UseShortRelativeDates` (@0x2b29b0, default true) is a
    /// separate switch.
    #[serde(default)]
    pub date_format: usize,
    /// Which name comparison the Sort menu uses, as the index of the three rows
    /// the original's Sort page offers (`FUN_001e5750` @0x1e5750):
    /// 0 Natural, 1 Alphabetical case insensitive, 2 Alphabetical case sensitive.
    #[serde(default)]
    pub name_sort: usize,
    /// The sort key the window last used, as an index into the four keys (Name,
    /// Size, Type, Modified). The original keeps a key *per folder* — the
    /// `sortRole`/`sortOrder` pair that `ViewProperties` @0x29fdd0 reads and
    /// saves — while this build keeps one for the window; the person asked for
    /// the choice to carry forward to the next start, and one default is what
    /// that means when a new folder may never have been opened before.
    #[serde(default)]
    pub sort_key: usize,
    /// Whether that key is descending. Size and Modified start descending
    /// (`DolphinView::defaultSortOrderForRole` @0x273910), so a fresh profile
    /// stores `false` here and still shows the newest file first after picking
    /// Modified.
    #[serde(default)]
    pub sort_desc: bool,
    /// Where the split divider was dragged to, 0..1 across the pane. The original
    /// feeds `QSplitter::setSizes` from its saved config (call sites 0x90e2a,
    /// 0x927f3, …), i.e. it remembers the drag rather than opening at a fixed
    /// width.
    #[serde(default = "half")]
    pub split_frac: f32,
    /// Reopen the tabs of the last run. The original's Startup page offers
    /// "Folders, tabs, and window state from last time" (0x137dc8) and saves it
    /// in `DolphinMainWindow::slotSaveSession`.
    #[serde(default = "yes")]
    pub restore_session: bool,
    /// One entry per open tab, in tab order.
    #[serde(default)]
    pub session: Vec<SavedTab>,
    /// Index of the tab that was in front.
    #[serde(default)]
    pub session_tab: usize,
    /// Inner size of the window on the last run, so the window opens the size it
    /// was left. Not used in screenshot mode, so captures stay identical.
    #[serde(default)]
    pub window_px: Option<[f32; 2]>,
    /// Where the window's top-left corner was on the last run, so it opens in
    /// the same place instead of wherever the window manager drops it. Added
    /// feature: the original restores its whole window state from the session
    /// (`DolphinMainWindow::slotSaveSession`), and a window that comes back at
    /// the same spot is part of that. Not used in screenshot mode.
    #[serde(default)]
    pub window_pos: Option<[f32; 2]>,
    /// The Shortcuts section of the navigation panel can be folded away.
    #[serde(default = "yes")]
    pub shortcuts_expanded: bool,
    /// Added feature: the two groups the original's panel shows under Places
    /// and leaves empty on a fresh profile — *Recent Files* and *Recent
    /// Locations*. Each holds rows here: the files the desktop's own recently
    /// used list names, and the folders this window walked through. The two
    /// fold away like the Shortcuts group and are remembered folded.
    #[serde(default = "yes")]
    pub recent_files_expanded: bool,
    #[serde(default = "yes")]
    pub recent_locations_expanded: bool,
    /// The *Network* group folds the same way. It is open on a fresh profile
    /// because the original shows its Remote group from the start.
    #[serde(default = "yes")]
    pub network_expanded: bool,
    /// Added feature, asked for by the person: the light palette instead of the
    /// dark one. Absent from an older settings file, it means dark — the palette
    /// the window has always drawn.
    #[serde(default)]
    pub light: bool,
    /// Added feature, asked for by the person: how deep the light palette's
    /// shading goes, 0–100 %. Written down because a slider that is back at 0
    /// after a restart is worse than no slider: a display with poor shadow
    /// separation would have to be corrected at every launch.
    #[serde(default)]
    pub shade: u8,
    /// Added feature: whether the *Network* group is drawn at all. A desktop
    /// with no shares shows an empty heading and one row of apology, so the
    /// menu carries a switch to take the group off the panel and put it back.
    #[serde(default = "yes")]
    pub network_visible: bool,
    /// Added feature: the *Added tools* group folds away like the other groups
    /// and starts folded. It is a list of tools rather than a place to walk to,
    /// so a new window shows the places first and the tools one click away.
    #[serde(default)]
    pub tools_expanded: bool,
    /// How many rows *Recent Files* asks the list for. The panel is narrow and
    /// a group taller than the window is not a shortcut, so it stops at eight.
    #[serde(default = "recent_rows")]
    pub recent_files_shown: usize,
    /// The folders walked through, newest first, kept between runs so the group
    /// has something in it on the first frame of a new window.
    #[serde(default)]
    pub recent_locations: Vec<PathBuf>,
    /// Show the keyboard hint next to each shortcut row.
    #[serde(default = "yes")]
    pub shortcut_keys: bool,
    /// Colours the person gave to rows of the Shortcuts panel, keyed by folder.
    #[serde(default)]
    pub shortcut_colors: Vec<ShortcutColor>,
    /// Added feature: the picture gallery was the last presentation used, and
    /// how big its thumbnails were, whether names and dimension captions were
    /// shown. The gallery's own size range is 50..400 px.
    #[serde(default)]
    pub pictures: bool,
    #[serde(default = "gallery_thumb")]
    pub thumb_px: f32,
    #[serde(default = "yes")]
    pub gallery_names: bool,
    #[serde(default)]
    pub gallery_dims: bool,
    /// Rounded corners on the gallery's thumbnails.
    #[serde(default = "yes")]
    pub gallery_round: bool,
    /// Slideshow interval in seconds and whether pictures cross-fade. Kept
    /// between runs so the show starts the way the person left it.
    #[serde(default = "slide_secs")]
    pub slide_secs: f32,
    #[serde(default = "yes")]
    pub slide_fade: bool,
    /// `GeneralSettings` key `ShowZoomSlider` (ctor @0x2b8630, `ItemBool(..., false)`,
    /// `this+0xa8`): the original's status-bar zoom slider is **off** in a fresh
    /// profile and is switched on from the status bar's own context menu
    /// (`FUN_001ec540` @0x1ec540 writes that key and shows or hides the slider
    /// widgets). This build ships it on, because the person asked for the
    /// bottom-right zoom control; the switch below is the original's.
    #[serde(default = "yes")]
    pub show_zoom_slider: bool,
    /// `ContentDisplaySettings` key `RecursiveDirectorySizeLimit` (ctor @0x2b2b70,
    /// `ItemUInt(..., 10)`, `this+0x1c`): how deep "size of contents" walks. The
    /// View → Display page's spin box allows 1..20 in steps of 1
    /// (`FUN_001e5750` @0x1e5750, `setRange(1, 20)`, `setSingleStep(1)`).
    #[serde(default = "size_limit")]
    pub size_limit: u32,
    /// Added feature: whether the prompt block (the recipe a picture was made
    /// with) is open or folded away. The person asked for it to collapse because
    /// it is not always needed; the choice is kept between runs so it stays as
    /// they left it.
    #[serde(default = "yes")]
    pub prompt_open: bool,
    /// The original's Confirmations page keeps three separate switches, one per
    /// destructive action (`ConfirmTrash`, `ConfirmDelete`, `ConfirmEmptyTrash`;
    /// the page is labelled "Ask for confirmation in Dolphin when:"). The
    /// rebuild stores the same three. `ConfirmTrash` ships **off** here because
    /// the person asked for a marked picture and a press of Delete to be the
    /// whole action; the original's own default for it is off as well
    /// (`KIO::WidgetsAskUserActionHandler::askUserDelete` @0x1a7980 in
    /// `libKF6KIOWidgets.so.6.31.0`, key at 0x21e8f0, default 0). `ConfirmDelete`
    /// and `ConfirmEmptyTrash` keep the original's defaults, both on.
    #[serde(default)]
    pub confirm_trash: bool,
    #[serde(default = "yes")]
    pub confirm_delete: bool,
    #[serde(default = "yes")]
    pub confirm_empty_trash: bool,
    /// `[PreviewSettings] MaximumSize` read by the previews page
    /// (`FUN_00217c30` @0x217c30): a file bigger than this many MiB gets no
    /// thumbnail. 0 is the spin box's "files of any size" special value, i.e. no
    /// cap, which is also this build's factory default.
    #[serde(default)]
    pub preview_max_mib: u32,
}

fn size_limit() -> u32 {
    crate::search::SIZE_LIMIT_DEFAULT
}

fn slide_secs() -> f32 {
    crate::gallery::SLIDE_DEFAULT
}

fn gallery_thumb() -> f32 {
    crate::gallery::THUMB_DEFAULT
}

fn yes() -> bool {
    true
}

fn recent_rows() -> usize {
    8
}

fn half() -> f32 {
    0.5
}

fn details() -> usize {
    2
}

fn default_zoom() -> [u8; 3] {
    // A config file that predates the `zoom` key must land on the same default
    // as a fresh profile: `DolphinView::updateDefaultZoomLevel` @0x27dd60 asks
    // each mode's settings object for `PreviewSize` while previews are shown
    // (Icons 64, Compact 48, Details 48) and for `IconSize` when they are off
    // (Icons 32, Compact 16, Details 16), then turns that size into a zoom level
    // with `ZoomLevelInfo::zoomLevelForIconSize` @0x2a63a0 → levels 4, 3, 3
    // with previews and 2, 0, 0 without.
    crate::theme::default_zoom(true)
}

fn default_zoom_plain() -> [u8; 3] {
    crate::theme::default_zoom(false)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            settings_version: SETTINGS_VERSION,
            bookmarks: Vec::new(),
            last_dir: String::new(),
            split: false,
            places_visible: true,
            info_panel: false,
            show_hidden: false,
            previews: true,
            // The original opens a fresh window in the icons view, at the zoom
            // whose size matches the mode's PreviewSize (DolphinView::
            // updateDefaultZoomLevel @0x27dd60). Measured on a fresh profile:
            // icons view, 64 px tiles.
            view: 0,
            zoom: crate::theme::default_zoom(true),
            zoom_plain: crate::theme::default_zoom(false),
            folders_first: true,
            places_autoscroll: true,
            date_format: 0,
            name_sort: 0,
            sort_key: 0,
            sort_desc: false,
            split_frac: 0.5,
            restore_session: true,
            session: Vec::new(),
            session_tab: 0,
            window_px: None,
            window_pos: None,
            shortcuts_expanded: true,
            recent_files_expanded: true,
            recent_locations_expanded: true,
            network_expanded: true,
            network_visible: true,
            tools_expanded: false,
            light: false,
            shade: 0,
            recent_files_shown: 8,
            recent_locations: Vec::new(),

            shortcut_keys: true,
            shortcut_colors: Vec::new(),
            pictures: false,
            thumb_px: crate::gallery::THUMB_DEFAULT,
            gallery_names: true,
            gallery_dims: false,
            gallery_round: true,
            slide_secs: crate::gallery::SLIDE_DEFAULT,
            slide_fade: true,
            // The original's factory value is false; this build keeps the slider
            // visible because the person asked for the bottom-right zoom control.
            show_zoom_slider: true,
            size_limit: crate::search::SIZE_LIMIT_DEFAULT,
            prompt_open: true,
            // The person's rule wins over the original's factory setting for the
            // Trash move: a marked picture and a press of Delete is the whole
            // action, no question in between. The original's default for that
            // switch is off too. A permanent delete and emptying the Trash keep
            // the original's defaults and do ask, because there is no way back.
            confirm_trash: false,
            confirm_delete: true,
            confirm_empty_trash: true,
            preview_max_mib: 0,
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::home_dir().join(".config"));
    base.join("dolphin-remake").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut config: Self = serde_json::from_str(&text).unwrap_or_default();
        config.migrate();
        config
    }

    /// Bring a settings file written by an older build up to this build's
    /// defaults. Only a value that was never a real choice is touched: before
    /// the person asked for 1.5 s, a fresh show started at exactly 1.0 s, so a
    /// file still holding 1.0 is assumed to be the old default rather than a
    /// deliberate pick. Any other interval survives.
    pub fn migrate(&mut self) {
        if self.settings_version < SETTINGS_VERSION {
            if self.settings_version == 0 && (self.slide_secs - 1.0).abs() < 0.001 {
                self.slide_secs = crate::gallery::SLIDE_DEFAULT;
            }
            self.settings_version = SETTINGS_VERSION;
        }
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// The folders of the tabs remembered from the last run, leaving out any that
    /// no longer exist. A session file can outlive a folder (an unmounted disk, a
    /// deleted directory), and the original's restore drops such a URL rather
    /// than opening an error page.
    pub fn remembered_tabs(&self) -> Vec<SavedTab> {
        self.session
            .iter()
            .filter(|t| t.path.is_dir())
            .cloned()
            .collect()
    }

    /// The colour of one shortcut row, if the person chose one.
    pub fn shortcut_color(&self, path: &Path) -> Option<[u8; 3]> {
        self.shortcut_colors
            .iter()
            .find(|c| c.path == path)
            .map(|c| c.rgb)
    }

    /// Store a colour for one shortcut row; `None` puts the row back to the
    /// normal place colour. The entry is removed rather than kept as a
    /// "no colour" record, so the file stays as small as the choices made.
    pub fn set_shortcut_color(&mut self, path: &Path, rgb: Option<[u8; 3]>) {
        self.shortcut_colors.retain(|c| c.path != path);
        if let Some(rgb) = rgb {
            self.shortcut_colors.push(ShortcutColor {
                path: path.to_path_buf(),
                rgb,
            });
        }
    }
}

/// Move one shortcut to an insertion slot in the list. `to` counts the gaps
/// between rows: 0 is above the first row, `list.len()` is below the last, and
/// `to` between two rows puts the dragged one in that gap.
///
/// This is the rule `KFilePlacesModel::movePlace(int from, int to)` @0x96220 in
/// libKF6KIOFileWidgets.so.6 follows: a slot equal to `from` or `from + 1` means
/// "it is already there" and the function returns without touching anything;
/// otherwise the dragged row lands at `to - 1` when it moved down and at `to`
/// when it moved up. Returns whether the order really changed.
/// Take a group apart: the container goes and its shortcuts step out into the
/// top level at the place it stood, in the order they were in.
pub fn dissolve_group(tree: &mut Vec<Bookmark>, index: usize) -> bool {
    if !tree.get(index).map(|b| b.group).unwrap_or(false) {
        return false;
    }
    let grp = tree.remove(index);
    for (k, child) in grp.children.into_iter().enumerate() {
        tree.insert(index + k, child);
    }
    true
}
