//! Dolphin-style file manager: window, toolbar, tabs, places, panes, status.

/// The name on the window title bar, in the task switcher and in the app id.
/// The original program declares its own name "Dolphin" (`@title` of its
/// KAboutData block at 0x12d350); this build carries its own name instead, as
/// the person asked. Everything that shows the app's name reads this one string.
pub const APP_NAME: &str = "DolphinCraft";

/// The release this build came from. `Cargo.toml`'s `version` is the number on
/// the GitHub release tag (`v0.1.3`), so About names the download the person
/// actually has — no network call, no guessing from a commit hash.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The one line About shows: the name, the release, and what the app is.
pub fn about_line() -> String {
    format!("{APP_NAME} {APP_VERSION} — a file manager with extra tools")
}

mod apps;
mod config;
mod foldercolor;
mod fsutil;
mod gallery;
mod imagemeta;
mod panels;
mod search;
mod selectable;
mod selftest;
mod state;
mod theme;
mod views;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Modifiers, Pos2, Rect,
    Sense, Stroke, Vec2, ViewportCommand,
};

use config::{Config, SavedTab};
use search::{DupScan, ThumbCache};
use state::{Clipboard, Pane, Request, ViewMode, TRASH_URI};
use theme::*;
use views::ViewCtx;

/// How many folders the navigation panel's *Recent Locations* group keeps. The
/// group is a way back to somewhere the person was, not a history browser; six
/// rows fit beside the other groups without pushing Devices off the panel.
pub const RECENT_LOCATIONS_MAX: usize = 6;

/// Remember that a tab was closed, so "Recently Closed Tabs" can put it back.
/// A folder already in the list moves to the newest end rather than appearing
/// twice, and the list stops at `cap` entries — the newest are the ones kept.
pub(crate) fn remember_closed_tab(list: &mut Vec<PathBuf>, path: PathBuf, cap: usize) {
    list.retain(|p| *p != path);
    list.push(path);
    let over = list.len().saturating_sub(cap);
    if over > 0 {
        list.drain(0..over);
    }
}

/// Put a folder at the newest end of *Recent Locations*. A folder already in the
/// list moves up instead of appearing twice, and the list stops at `cap`, so the
/// group always shows the most recent walks first. Split out from the window so
/// the rule can be checked without one.
pub(crate) fn note_location(list: &mut Vec<PathBuf>, path: &Path, cap: usize) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(cap);
}

pub struct Tab {
    pub panes: Vec<Pane>,
    pub active: usize,
}

// ---------------------------------------------------------------------------
// Toolbar geometry
//
// The original puts the sort control in the same row as the location bar and
// lets the bar take whatever room is left. Doing that with a fixed guess for
// the right-hand cluster is what drew the sort label on top of the bar, so the
// cluster's width is measured from its own controls instead.
// ---------------------------------------------------------------------------

/// One toolbar control.
pub const TOOLBAR_ICON_W: f32 = 30.0;
/// Gap between controls in the right-hand cluster.
pub const TOOLBAR_ITEM_GAP: f32 = 2.0;
/// Breathing room between the location bar's frame and the cluster.
pub const TOOLBAR_CLUSTER_GAP: f32 = 8.0;
/// Below this the location bar is not worth drawing at all.
pub const TOOLBAR_BAR_USABLE: f32 = 70.0;
/// Widest the bar may become before the cluster would start squeezing.
pub const TOOLBAR_BAR_MIN: f32 = 110.0;
/// Seven icon buttons, the two 4 px separators (each also costs one item gap)
/// and the eight item gaps between the nine things in the cluster.
pub const TOOLBAR_CLUSTER_FIXED: f32 =
    7.0 * TOOLBAR_ICON_W + 2.0 * (4.0 + TOOLBAR_ITEM_GAP) + 8.0 * TOOLBAR_ITEM_GAP;

/// Padding between a toolbar button's icon and the word beside it.
pub const TOOLBAR_LABEL_PAD: f32 = 6.0;
/// What the two words cost on top of the icons they sit beside.
pub const TOOLBAR_LABEL_EXTRA: f32 = 2.0 * TOOLBAR_LABEL_PAD;

/// How much of the sort control is shown. It gives way, label first, so the
/// location bar keeps a workable width in a narrow window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortControl {
    Label,
    Icon,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolbarMetrics {
    pub sort: SortControl,
    /// Whether the two verbs ("Find", "Menu") are spelled out.
    pub labeled: bool,
    pub cluster_w: f32,
    pub bar_w: f32,
}

/// Split `avail` pixels between the location bar and the buttons. `label_w` is
/// the measured width of the sort key's text; `words_w` the measured width of
/// the three toolbar words. The invariant this function keeps is
/// `bar_w + TOOLBAR_CLUSTER_GAP + cluster_w <= avail`, so the two can never be
/// drawn over each other, and it gives things up in a fixed order: the sort
/// key's name, then the sort button, and only then the three words — because a
/// row of bare glyphs is the thing the person could not read, so it is the last
/// thing to go.
pub fn toolbar_metrics(avail: f32, label_w: f32, show_bar: bool, words_w: f32) -> ToolbarMetrics {
    let icons = TOOLBAR_CLUSTER_FIXED;
    let words = icons + words_w + TOOLBAR_LABEL_EXTRA;
    // The sort button carries its key's name plus the style's 7 px side padding.
    let sort_label = label_w + 2.0 * 7.0;
    let floor = if show_bar { TOOLBAR_BAR_MIN } else { 0.0 };
    let fits = |c: f32| avail - c - TOOLBAR_CLUSTER_GAP >= floor;
    let width = |labeled: bool, sort: SortControl| {
        let base = if labeled { words } else { icons };
        match sort {
            SortControl::Label => base + sort_label,
            SortControl::Icon => base,
            SortControl::Hidden => base - TOOLBAR_ICON_W - TOOLBAR_ITEM_GAP,
        }
    };

    let ladder = [
        (true, SortControl::Label),
        (true, SortControl::Icon),
        (true, SortControl::Hidden),
        (false, SortControl::Label),
        (false, SortControl::Icon),
        (false, SortControl::Hidden),
    ];
    let (labeled, sort) = ladder
        .iter()
        .copied()
        .find(|&(l, s)| fits(width(l, s)))
        .unwrap_or((false, SortControl::Hidden));
    let cluster_w = width(labeled, sort);
    let bar_w = if show_bar {
        (avail - cluster_w - TOOLBAR_CLUSTER_GAP).max(0.0)
    } else {
        0.0
    };
    ToolbarMetrics {
        sort,
        labeled,
        cluster_w,
        bar_w,
    }
}

pub enum ConfirmAction {
    Trash(Vec<PathBuf>),
    Delete(Vec<PathBuf>),
    EmptyTrash,
    Restore(Vec<PathBuf>),
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub action: ConfirmAction,
}

/// What the last Move to Trash put away, so the little toast can offer to put it
/// back. The person asked for no question before a delete; a way back afterwards
/// is what replaces it. The original has an Undo for trashing too (`This undoes
/// the last change you made to files or folders`), so this stays in its spirit.
pub struct UndoTrash {
    /// The Trash records this action created, in `~/.local/share/Trash/files`.
    pub records: Vec<PathBuf>,
    pub at: Instant,
}

/// How long the Undo offer stays up. The toast itself lives 4 s; an offer that
/// short would be gone before the eye finds it.
pub const UNDO_SECS: f32 = 8.0;

/// How many closed tabs the menu remembers. The original's list is a
/// `KRecentFilesAction` (its subtitle "Recently Closed Tabs", 0x12d8ff in
/// /usr/bin/dolphin, feeding a `DolphinRecentTabsMenu` at 0x13d9a0), which keeps
/// ten entries by default; this build keeps the same ten.
pub const CLOSED_TAB_MEMORY: usize = 10;

pub struct Props {
    pub paths: Vec<PathBuf>,
    pub checksum: Option<String>,
    pub busy: bool,
    pub rx: Option<Receiver<Option<String>>>,
    pub folder_size: Option<u64>,
}

pub struct Bulk {
    pub paths: Vec<PathBuf>,
    pub pattern: String,
    pub start: u32,
    pub error: Option<String>,
}

pub struct Dups {
    pub scan: DupScan,
}

pub struct Quick {
    pub path: PathBuf,
    pub text: Option<String>,
}

/// The chooser behind "Other Application…": every program the desktop knows,
/// narrowed as the person types, with the original's own tick box ("Remember
/// application association for all files of type …") underneath.
pub struct OpenWith {
    /// What is being opened, in the order the view had it.
    pub paths: Vec<PathBuf>,
    /// The type the tick box names.
    pub mime: String,
    /// The text that narrows the list.
    pub filter: String,
    /// Put the choice into the desktop's own list of preferences.
    pub remember: bool,
    /// The row picked with the arrow keys, or the only match when there is one.
    pub chosen: usize,
    pub focus: bool,
}

pub struct NewFolder {
    pub name: String,
    pub focus: bool,
}

/// The little editor behind "Add Shortcut" / "Rename Shortcut": a name and the
/// folder the shortcut points at.
pub struct ShortcutEdit {
    pub name: String,
    pub path: String,
    pub focus: bool,
    pub error: Option<String>,
    /// Set when an existing shortcut is being renamed instead of added.
    pub original: Option<PathBuf>,
}

pub struct Shot {
    pub path: PathBuf,
    pub requested: bool,
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// Added: the little card that explains what Find, Open and Menu each do,
    /// so the two icons stop being a guessing game.
    pub guide_open: bool,
    /// `--menu`: open the app menu on the first frame so a picture of it can be
    /// taken. A click on the button opens it in normal use.
    pub open_menu_once: bool,
    /// `--sort-menu`: open the Sort By popup on the first frame so a picture of it
    /// can be taken. A click on the sort control opens it in normal use.
    pub open_sort_once: bool,
    pub split: bool,
    pub places_visible: bool,
    pub info_panel: bool,
    pub clipboard: Clipboard,
    pub thumbs: ThumbCache,
    pub config: Config,
    pub reqs: Vec<Request>,
    pub confirm: Option<Confirm>,
    pub props: Option<Props>,
    pub bulk: Option<Bulk>,
    pub dups: Option<Dups>,
    pub quick: Option<Quick>,
    pub new_folder: Option<NewFolder>,
    /// Added feature, as asked for this round: the chooser behind "Open With…",
    /// and the folders of the tabs that were closed, newest last, so a tab
    /// closed by mistake can come back (`Recently Closed Tabs`, 0x12d8ff in
    /// /usr/bin/dolphin).
    pub open_with: Option<OpenWith>,
    pub closed_tabs: Vec<PathBuf>,
    pub shortcut_edit: Option<ShortcutEdit>,
    pub toast: Option<(String, Instant)>,
    /// The Undo offer that follows the last Move to Trash.
    pub undo_trash: Option<UndoTrash>,
    pub folder_sizes: HashMap<PathBuf, u64>,
    pub size_rx: Receiver<(PathBuf, u64)>,
    pub size_tx: Sender<(PathBuf, u64)>,
    pub shot: Option<Shot>,
    /// Added feature: the picture viewer, when one is open, and the loader that
    /// feeds it full-size pictures.
    pub viewer: Option<gallery::Viewer>,
    pub big: gallery::BigCache,
    /// A picture asked for on the command line, opened on the first frame.
    pub pending_viewer: Option<(PathBuf, bool)>,
    /// `--trash-demo <folder>`: move that folder's contents to the Trash once, so
    /// a captured window shows the toast and its Undo button. Nothing is deleted
    /// for good, and nothing happens unless the flag is given.
    pub pending_trash_demo: Option<PathBuf>,
    /// `--open-with <file>`: open the chooser on that file once the window is
    /// up, so the dialog can be photographed without a mouse.
    pub pending_open_with: Option<PathBuf>,
    /// What each picture's own file says about how it was generated. Read once
    /// per file and kept, so the information panel does not parse a PNG every
    /// frame.
    pub image_meta: HashMap<PathBuf, Option<imagemeta::ImageMeta>>,
    /// Open the viewer's comparison as soon as that picture is up.
    pub pending_compare: Option<gallery::CompareMode>,
    pub frames: u64,
    /// `--details`: open the viewer's details panel on the first picture, so the
    /// panel can be photographed without a mouse. Off unless asked for: the
    /// panel starts closed.
    pub open_details_at_start: bool,
    pub config_dirty: bool,
    pub next_pane_id: usize,
    pub split_frac: f32,
    pub filter_focus: bool,
    /// Which of the person's own shortcut rows is being dragged, if any.
    /// The value is the index into `config.bookmarks`.
    pub shortcut_drag: Option<usize>,
    /// What the remembered session looked like on the last frame that was
    /// written to disk. Compared about once a second, so opening a tab,
    /// closing one or walking into another folder reaches the settings file
    /// even though none of those is a "setting" the menus own. Empty until the
    /// first frame.
    pub session_sig: String,
    /// The whole settings file as it stood when it was last written, compared
    /// with the live window about twice a second. Every choice the window keeps
    /// is in this string, so a switch that forgot to say "I changed" is still
    /// written — the view buttons used to be exactly that case, and a choice
    /// made there was lost if the window was not closed tidily.
    pub settings_sig: String,
    /// Added feature: the files the navigation panel's *Recent Files* group
    /// shows, read from the desktop's own recently used list. Kept with the
    /// moment they were read, so a window open all day does not ask the disk
    /// for that list on every frame.
    pub recent_files: Vec<fsutil::Entry>,
    pub recent_files_at: Option<Instant>,
}

/// The settings as one comparable string. Two snapshots that differ mean the
/// window's choices changed since the last write, which is the whole test for
/// "is there anything to save" — so a choice cannot be lost just because the
/// button that made it did not say it had changed.
pub(crate) fn settings_signature(cfg: &Config) -> String {
    serde_json::to_string(cfg).unwrap_or_default()
}

/// Where the window should open: the place it was left, unless that place is
/// off any plausible screen (a monitor that has since been unplugged), in which
/// case the window manager picks, as it did before.
pub(crate) fn startup_position(cfg: &Config) -> Option<[f32; 2]> {
    let pos = cfg.window_pos?;
    let sane = |v: f32| v.is_finite() && v >= -32_000.0 && v <= 32_000.0;
    if sane(pos[0]) && sane(pos[1]) {
        Some(pos)
    } else {
        None
    }
}

/// Put every display choice back to its factory default. Used by the menu's
/// "Forget remembered settings", which is how the person can get back to a
/// plain window without hunting for the switches they once moved.
pub(crate) fn apply_defaults(pane: &mut Pane) {
    let d = Config::default();
    pane.view = ViewMode::from_index(d.view.min(2));
    pane.zoom = d.zoom;
    pane.zoom_plain = d.zoom_plain;
    pane.show_hidden = d.show_hidden;
    pane.previews = d.previews;
    pane.folders_first = d.folders_first;
    pane.name_sort = state::NameSort::from_index(d.name_sort);
    pane.sort = state::SortKey::from_index(d.sort_key);
    pane.asc = !d.sort_desc;
    pane.pictures = false;
    pane.thumb_px = gallery::clamp_thumb(d.thumb_px);
    pane.gallery_names = d.gallery_names;
    pane.gallery_dims = d.gallery_dims;
    pane.gallery_round = d.gallery_round;
    pane.filter_on = false;
    pane.filter.clear();
    pane.search = None;
    pane.refilter();
}

/// A pane set up from the saved settings, so a tab brought back by the session
/// starts with the view, zoom and sort the window was left with. The original
/// reads the same set out of each folder's view properties and the mode settings
/// (`DolphinView::updateDefaultZoomLevel` @0x27dd60, `ViewProperties` @0x29fdd0).
pub fn pane_from_config(id: usize, path: std::path::PathBuf, cfg: &Config) -> Pane {
    let mut pane = Pane::new(id, path);
    pane.view = ViewMode::from_index(cfg.view.min(2));
    pane.zoom = cfg.zoom;
    pane.zoom_plain = cfg.zoom_plain;
    pane.show_hidden = cfg.show_hidden;
    pane.previews = cfg.previews;
    pane.folders_first = cfg.folders_first;
    pane.name_sort = state::NameSort::from_index(cfg.name_sort);
    pane.pictures = cfg.pictures;
    pane.thumb_px = gallery::clamp_thumb(cfg.thumb_px);
    pane.gallery_names = cfg.gallery_names;
    pane.gallery_dims = cfg.gallery_dims;
    pane.gallery_round = cfg.gallery_round;
    pane.sort = state::SortKey::from_index(cfg.sort_key);
    pane.asc = !cfg.sort_desc;
    if cfg.show_hidden {
        // `Pane::new` listed the folder without the hidden entries; list it
        // again with them, then put the saved sort back on the new listing.
        pane.reload();
    }
    pane.sort_entries();
    pane
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        start_dir: Option<PathBuf>,
        shot: Option<PathBuf>,
        start_zoom: Option<u8>,
        start_view: Option<usize>,
        keep_settings: bool,
        start_viewer: Option<(PathBuf, bool)>,
        start_pictures: bool,
        start_thumbs: Option<f32>,
        start_compare: Option<gallery::CompareMode>,
        start_dims: bool,
        start_find: Option<String>,
        start_deep: bool,
        start_guide: bool,
        start_menu: bool,
        open_details_at_start: bool,
        start_below: bool,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        if shot.is_some() {
            // The picture is taken without a pointer of our own, so whatever
            // the cursor happens to rest on would otherwise leave a tooltip
            // floating in the middle of the saved window.
            cc.egui_ctx.style_mut(|s| s.interaction.tooltip_delay = 1.0e6);
        }
        // A captured picture must look the same every time, so it starts from
        // factory settings rather than from whatever the last session left
        // behind (a zoom level, a split view, a panel). `--keep-settings` asks
        // for the saved settings instead, so the picture shows the person's own
        // shortcuts, colours and layout.
        let fresh = shot.is_some() && !keep_settings;
        let cfg = if fresh {
            Config::default()
        } else {
            Config::load()
        };
        let mut next_id = 1usize;
        // Whether the folder came from the command line: a folder named there
        // wins over the saved session, so `dolphincraft ~/Pictures` opens just
        // that folder rather than every tab of the last run.
        let from_cli = start_dir.is_some();
        let dir = start_dir
            .or_else(|| {
                // A captured picture should show the same thing every time, so
                // the remembered folder is skipped in screenshot mode.
                if fresh {
                    return None;
                }
                let p = PathBuf::from(&cfg.last_dir);
                if p.is_dir() {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap_or_else(fsutil::home_dir);
        let mut pane = pane_from_config(next_id, dir, &cfg);
        next_id += 1;
        if let Some(v) = start_view {
            pane.view = ViewMode::from_index(v.min(2));
        }
        if let Some(z) = start_zoom {
            pane.set_zoom_level(z);
        }
        // Added feature: the gallery's own last state, restored like the rest of
        // the window's furniture.
        pane.pictures = cfg.pictures || start_pictures;
        // `--below`: the wall looking inside the folders below, so the switch
        // can be photographed without a mouse. Only with the wall up.
        if start_below && pane.pictures {
            pane.set_pictures_below(true);
        }
        if let Some(text) = start_find.clone() {
            pane.filter_on = true;
            pane.filter = text;
            pane.find_deep = start_deep;
            // The listing is narrowed here and not only on the next keystroke,
            // so a find handed over on the command line shows the matches.
            pane.refilter();
            if start_deep && !pane.filter.trim().is_empty() {
                pane.start_search();
            }
        }
        pane.thumb_px = gallery::clamp_thumb(start_thumbs.unwrap_or(cfg.thumb_px));
        if start_dims {
            // `--dims` starts the wall with the size line already showing.
            pane.gallery_dims = true;
        }
        pane.sort_entries();

        let shot_mode = shot.is_some();
        let (size_tx, size_rx) = channel();
        let mut thumbs = ThumbCache::new();
        // `[PreviewSettings] MaximumSize`, in MiB; 0 = files of any size.
        thumbs.set_max_mib(cfg.preview_max_mib);
        // Added feature asked for by the person: the tabs of the last run come
        // back, with the tab that was in front in front again. A folder handed
        // over on the command line, a find or the viewer takes precedence, and a
        // captured picture always starts from one folder so it stays the same
        // picture. The original's Startup page offers the same choice —
        // "Folders, tabs, and window state from last time" (0x137dc8).
        let mut tabs = vec![Tab {
            panes: vec![pane],
            active: 0,
        }];
        let mut active_tab = 0usize;
        if !shot_mode && !from_cli && cfg.restore_session && start_find.is_none() && start_viewer.is_none() {
            let remembered = cfg.remembered_tabs();
            if !remembered.is_empty() {
                tabs = remembered
                    .into_iter()
                    .map(|tab| {
                        let id = next_id;
                        next_id += 1;
                        let mut pane = pane_from_config(id, tab.path, &cfg);
                        // Each tab keeps the sort it was showing. A session file
                        // from before tabs carried their own sort falls back to the
                        // window's sort.
                        if let Some(key) = tab.sort_key {
                            pane.sort = state::SortKey::from_index(key);
                            pane.asc = !tab.sort_desc.unwrap_or(false);
                            pane.sort_entries();
                        }
                        Tab {
                            panes: vec![pane],
                            active: 0,
                        }
                    })
                    .collect();
                active_tab = cfg.session_tab.min(tabs.len() - 1);
            }
        }
        let saved_split_frac = cfg.split_frac.clamp(0.15, 0.85);
        let mut app = App {
            tabs,
            active_tab,
            // `--guide` opens the card on the first frame, which is how the
            // screenshot of it gets taken (F1 and the menu entry set this too).
            guide_open: start_guide,
            open_menu_once: start_menu,
            open_sort_once: false,
            // A start is always one view. Split is chosen in the window (F3,
            // the menu switch, or "In the other pane"), and is not carried into
            // the next start: an app that opens as two folders every time makes
            // the person undo a choice they did not make this run.
            split: false,
            places_visible: cfg.places_visible,
            info_panel: cfg.info_panel,
            clipboard: Clipboard::new(),
            thumbs,
            config: cfg,
            reqs: Vec::new(),
            confirm: None,
            props: None,
            bulk: None,
            dups: None,
            quick: None,
            new_folder: None,
            open_with: None,
            closed_tabs: Vec::new(),
            shortcut_edit: None,
            toast: None,
            undo_trash: None,
            folder_sizes: HashMap::new(),
            size_rx,
            size_tx,
            shot: shot.map(|p| Shot {
                path: p,
                requested: false,
            }),
            viewer: None,
            big: gallery::BigCache::new(),
            pending_viewer: start_viewer,
            pending_trash_demo: None,
            pending_open_with: None,
            image_meta: HashMap::new(),
            pending_compare: start_compare,
            frames: 0,
            open_details_at_start,
            config_dirty: false,
            next_pane_id: next_id,
            split_frac: saved_split_frac,
            session_sig: String::new(),
            settings_sig: String::new(),
            recent_files: Vec::new(),
            recent_files_at: None,
            filter_focus: false,
            shortcut_drag: None,
        };
        // Nothing to restore for the panes: a start is one view, whatever the
        // last window was left as. Screenshot mode is a plain single view too,
        // so the capture is the same every time.
        if shot_mode {
            app.split = false;
        }
        // The snapshot to compare against: what was just opened. Nothing is
        // written until a choice really changes something.
        app.settings_sig = settings_signature(&app.config_from_state(None));
        app
    }

    pub fn pane(&self) -> &Pane {
        let tab = &self.tabs[self.active_tab];
        &tab.panes[tab.active]
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        let tab = &mut self.tabs[self.active_tab];
        let idx = tab.active;
        &mut tab.panes[idx]
    }

    pub fn other_pane_mut(&mut self) -> Option<&mut Pane> {
        if !self.split {
            return None;
        }
        let tab = &mut self.tabs[self.active_tab];
        if tab.panes.len() < 2 {
            return None;
        }
        let idx = 1 - tab.active;
        Some(&mut tab.panes[idx])
    }

    pub fn ensure_split(&mut self) {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let zoom = self.config.zoom;
        let view = self.pane().view;
        let tab = &mut self.tabs[self.active_tab];
        if tab.panes.len() == 1 {
            let path = tab.panes[0].path.clone();
            let mut pane = Pane::new(id, path);
            pane.view = view;
            pane.zoom = zoom;
            tab.panes.push(pane);
        }
        self.split = true;
    }

    pub fn unsplit(&mut self) {
        self.split = false;
        let tab = &mut self.tabs[self.active_tab];
        if tab.panes.len() > 1 {
            tab.panes.truncate(1);
            tab.active = 0;
        }
    }

    pub fn new_tab_public(&mut self, path: PathBuf) {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = Pane::new(id, path);
        pane.view = ViewMode::from_index(self.config.view);
        pane.zoom = self.config.zoom;
        pane.show_hidden = self.config.show_hidden;
        pane.previews = self.config.previews;
        pane.name_sort = state::NameSort::from_index(self.config.name_sort);
        pane.folders_first = self.config.folders_first;
        // A new tab opens sorted the way the window last was, rather than dropping
        // back to Name / ascending and overwriting the remembered sort when the
        // session is saved.
        pane.sort = state::SortKey::from_index(self.config.sort_key);
        pane.asc = !self.config.sort_desc;
        pane.sort_entries();
        self.tabs.push(Tab {
            panes: vec![pane],
            active: 0,
        });
        self.active_tab = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        // Remember where the tab was, so "Recently Closed Tabs" can put it
        // back. The original keeps the same list (it is a `KRecentFilesAction`
        // under the menu's "Recently Closed Tabs", 0x12d8ff).
        if let Some(path) = self.tabs.get(idx).map(|t| t.panes[t.active].path.clone()) {
            remember_closed_tab(&mut self.closed_tabs, path, CLOSED_TAB_MEMORY);
        }
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    pub fn active_pane_index(&self) -> usize {
        self.tabs[self.active_tab].active
    }

    // -----------------------------------------------------------------------
    // Chrome
    // -----------------------------------------------------------------------

    fn top_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(46.0)
            .frame(
                egui::Frame::new()
                    .fill(TOOLBAR_BG)
                    .inner_margin(Margin::symmetric(8, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let (logo_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                    theme::draw_kind_icon(
                        ui.painter(),
                        logo_rect.shrink(2.0),
                        fsutil::Kind::Folder,
                        false,
                    );
                    ui.add_space(2.0);

                    let (can_back, can_fwd) = {
                        let p = &self.tabs[self.active_tab].panes[self.tabs[self.active_tab].active];
                        (!p.back.is_empty(), !p.forward.is_empty())
                    };
                    if toolbar_button(ui, "back", can_back).clicked() {
                        self.pane_mut().go_back();
                    }
                    if toolbar_button(ui, "forward", can_fwd).clicked() {
                        self.pane_mut().go_forward();
                    }
                    if toolbar_button(ui, "up", true).clicked() {
                        self.pane_mut().go_up();
                    }
                    if toolbar_button(ui, "refresh", true).clicked() {
                        self.pane_mut().reload();
                    }
                    ui.add_space(6.0);

                    // The bar and the right-hand cluster are measured, never guessed
                    // at: the bar gets exactly the room the cluster leaves, so the
                    // sort control can never be drawn on top of the location bar.
                    let show_bar = !self.split;
                    let avail = ui.available_width();
                    let sort_label = format!("Sort: {}", self.pane().sort.label());
                    let label_w = ui.fonts_mut(|f| {
                        f.layout_no_wrap(sort_label.clone(), FontId::proportional(12.0), TEXT)
                            .size()
                            .x
                    });
                    // The two words the row spells out when it has room. They
                    // are measured here so the bar and the buttons still never
                    // share pixels.
                    let words_w = ui.fonts_mut(|f| {
                        ["Find", "Menu"]
                            .iter()
                            .map(|t| {
                                f.layout_no_wrap(
                                    t.to_string(),
                                    FontId::proportional(12.0),
                                    TEXT,
                                )
                                .size()
                                .x
                            })
                            .sum::<f32>()
                    });
                    let metrics = toolbar_metrics(avail, label_w, show_bar, words_w);
                    let bar_w = metrics.bar_w;
                    if show_bar && bar_w >= TOOLBAR_BAR_USABLE {
                        let tab = &mut self.tabs[self.active_tab];
                        let idx = tab.active;
                        // The field's outline is painted at a fixed rect and its
                        // contents are clipped to it. A long path used to make the
                        // content (and with it the frame) grow past the space left
                        // for it, which is how it reached under the sort control.
                        let (bar_rect, _) =
                            ui.allocate_exact_size(Vec2::new(bar_w, 30.0), Sense::hover());
                        ui.painter().rect(
                            bar_rect,
                            CornerRadius::same(4),
                            HEADER_BG,
                            Stroke::new(1.0, BORDER),
                            egui::StrokeKind::Inside,
                        );
                        let inner = bar_rect.shrink2(Vec2::new(7.0, 2.0));
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(inner)
                                .layout(Layout::left_to_right(Align::Center)),
                            |ui| {
                                ui.set_clip_rect(bar_rect.shrink(1.0));
                                ui.set_min_height(22.0);
                                tab.panes[idx].breadcrumb_ui(ui, &mut self.reqs);
                            },
                        );
                    } else if show_bar {
                        // Too little room left for a readable location bar: leave the
                        // space empty rather than squeezing a field in under the cluster.
                        ui.add_space(bar_w.max(0.0));
                    }

                    let mut sort_rect: Option<Rect> = None;
                    let labeled = metrics.labeled;
                    // Two verbs sit at the far right and say what they do: Find
                    // (narrow what you see) and Menu (change the app). Everything
                    // left of them is layout — sorting and the three views — which
                    // is why they are grouped and only these two carry a word.
                    let cluster_inner = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let menu = toolbar_word_button(ui, "menu", labeled.then_some("Menu"), false, true);
                        menu.clone().on_hover_text(
                            "Menu — the app's own settings and tools. It never touches your files.",
                        );
                        // A menu button opens on an ordinary click. The
                        // right-click-only popup it used to carry made the
                        // button look dead, which is half of why the three
                        // glyphs were unreadable.
                        let mut popup = egui::Popup::menu(&menu);
                        if self.open_menu_once {
                            self.open_menu_once = false;
                            egui::Popup::open_id(
                                ui.ctx(),
                                egui::Popup::default_response_id(&menu),
                            );
                        }
                        // The menu is long — New, Tools, View, Starting, Deleting,
                        // Sort by, Help — and in a window that is not maximised it
                        // used to run off the bottom of the screen with no way to
                        // reach the last rows. It scrolls now, capped to the height
                        // of the window itself so it fits whether egui opens it
                        // below the button or flips it above.
                        popup.show(|ui| {
                            let limit = (ui.ctx().content_rect().height() - 96.0).max(120.0);
                            egui::ScrollArea::vertical()
                                .max_height(limit)
                                .auto_shrink([false, true])
                                // Shown even when the pointer is elsewhere: a menu
                                // that is quietly cut at the bottom is the fault
                                // being fixed here, so the bar has to say "there is
                                // more" without waiting for a hover.
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .show(ui, |ui| self.app_menu(ui));
                        });

                        let find_on = self.pane().filter_on;
                        if toolbar_word_button(ui, "search", labeled.then_some("Find"), find_on, true)
                            .on_hover_text(
                                "Find — show only items whose name matches: this folder as you type, every subfolder when you choose it (Ctrl+F)",
                            )
                            .clicked()
                        {
                            let on = !self.pane().filter_on;
                            let pane = self.pane_mut();
                            pane.filter_on = on;
                            if !on {
                                pane.filter.clear();
                                pane.search = None;
                                pane.find_deep = false;
                                pane.refilter();
                            }
                            self.filter_focus = on;
                        }
                        ui.add_space(4.0);
                        // Added feature: the picture gallery, one button beside
                        // the three view modes it stands in for.
                        let pictures = self.pane().pictures;
                        if toolbar_button_active(ui, "pictures", pictures)
                            .on_hover_text("Pictures — thumbnails of this folder (F4)")
                            .clicked()
                        {
                            let pane = self.pane_mut();
                            let on = !pane.pictures;
                            pane.set_pictures(on);
                            self.config.pictures = pane.pictures;
                            self.config_dirty = true;
                        }
                        for mode in [ViewMode::Details, ViewMode::Compact, ViewMode::Icons] {
                            let glyph = match mode {
                                ViewMode::Icons => "grid",
                                ViewMode::Compact => "list",
                                ViewMode::Details => "details",
                            };
                            let active = self.pane().view == mode;
                            if toolbar_button_active(ui, glyph, active)
                                .on_hover_text(format!("{} view", mode.label()))
                                .clicked()
                            {
                                let zoom = self.config.zoom;
                                let pane = self.pane_mut();
                                pane.view = mode;
                                pane.zoom = zoom;
                                self.config.view = mode.index();
                                // The view is one of the choices the next start
                                // opens with, so it is written straight away; the
                                // whole-settings comparison would catch it too.
                                self.config_dirty = true;
                            }
                        }
                        ui.add_space(4.0);
                        let r = match metrics.sort {
                            SortControl::Label => {
                                Some(self.sort_control(ui, Some((&sort_label, label_w))))
                            }
                            SortControl::Icon => Some(self.sort_control(ui, None)),
                            SortControl::Hidden => None,
                        };
                        sort_rect = r.map(|r| r.rect);
                    });
                    if std::env::var_os("FILE_CRAFT_TOOLBAR_DEBUG").is_some() {
                        eprintln!(
                            "toolbar avail={avail:.1} bar_w={bar_w:.1} cluster_w={:.1} sort={:?} cluster_rect={:?} sort_rect={:?}",
                            metrics.cluster_w, metrics.sort, cluster_inner.response.rect, sort_rect
                        );
                    }
                });
            });
    }

    /// The sort control: an icon-only button in a narrow window, otherwise the
    /// current key's name next to a downward arrow, opening the same menu the
    /// original shows. Its width is fixed by the caller's measurement so the
    /// location bar is never drawn underneath it.
    fn sort_control(&mut self, ui: &mut egui::Ui, label: Option<(&str, f32)>) -> egui::Response {
        let (width, tip) = match label {
            Some((_, text_w)) => (text_w + 2.0 * 7.0, format!("Sort by: {}", self.pane().sort.label())),
            None => (
                TOOLBAR_ICON_W,
                format!("Sort by: {}", self.pane().sort.label()),
            ),
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, TOOLBAR_ICON_W), Sense::click());
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), ROW_HOVER);
        }
        match label {
            Some((text, _)) => {
                let arrow = 9.0;
                let text_rect = Rect::from_min_max(
                    rect.min,
                    Pos2::new(rect.right() - arrow - 2.0, rect.bottom()),
                );
                ui.painter().text(
                    Pos2::new(text_rect.left() + 7.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    FontId::proportional(12.0),
                    TEXT,
                );
                theme::draw_glyph(
                    ui.painter(),
                    Rect::from_center_size(
                        Pos2::new(rect.right() - 7.0 - arrow * 0.5, rect.center().y),
                        Vec2::splat(16.0),
                    ),
                    "dropdown",
                    TEXT_DIM,
                );
            }
            None => theme::draw_glyph(ui.painter(), rect, "sort", TEXT),
        }
        let resp = resp.on_hover_text(tip);
        if self.open_sort_once {
            self.open_sort_once = false;
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
        }
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| self.sort_menu(ui));
        resp
    }

    fn sort_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(190.0);
        // The original's Sort By menu, read out of `createActions` @0x28e380 in
        // libdolphinprivate.so.6: a radio group of sort keys (one `KToggleAction`
        // per role, built by `createFileItemRolesActionGroup` @0x28d3f0 with the
        // role as the action's data), a separator, a second exclusive pair
        // Ascending / Descending, then the Folders First switch. Choosing a key
        // never reverses the order — `slotSortTriggered` @0x291f30 only records the
        // choice — so the key that is already ticked does nothing, and the
        // direction is always this pair.
        {
            let pane = self.pane_mut();
            let mut changed = false;
            for key in [
                state::SortKey::Name,
                state::SortKey::Size,
                state::SortKey::Type,
                state::SortKey::Modified,
            ] {
                let checked = pane.sort == key;
                if ui.selectable_label(checked, key.label()).clicked() {
                    changed |= pane.choose_sort(key);
                    ui.close();
                }
            }
            ui.separator();
            if ui.selectable_label(pane.asc, "Ascending").clicked() {
                changed |= pane.set_sort_dir(false);
                ui.close();
            }
            if ui.selectable_label(!pane.asc, "Descending").clicked() {
                changed |= pane.set_sort_dir(true);
                ui.close();
            }
            ui.separator();
            // The original's own words: "Folders First", context
            // `@action:inmenu Sort`. The switch is a property of the folder
            // (`DolphinView::updateSortFoldersFirst` @0x27d7c0 writes it into the
            // folder's view properties), and a folder that has never been given one
            // starts with folders first (`ViewProperties` ctor @0x29fdd0 stores
            // 0,1,0 at +8,+9,+10: order ascending, folders first, hidden last off).
            if ui
                .checkbox(&mut pane.folders_first, "Folders First")
                .changed()
            {
                pane.sort_entries();
                changed = true;
            }
            if changed {
                // A sort chosen now is the sort the next start opens with.
                self.config_dirty = true;
            }
        }
        ui.separator();
        // The original's Sort page (`FUN_001e5750` @0x1e5750) offers three name
        // comparisons as radio rows. They live in a submenu here so the four
        // sort keys stay one click away.
        ui.menu_button("Name comparison", |ui| {
            let mut changed = false;
            {
                let pane = self.pane_mut();
                for mode in [
                    state::NameSort::Natural,
                    state::NameSort::CaseInsensitive,
                    state::NameSort::CaseSensitive,
                ] {
                    if ui
                        .selectable_label(pane.name_sort == mode, mode.label())
                        .clicked()
                    {
                        if pane.name_sort != mode {
                            pane.name_sort = mode;
                            pane.sort_entries();
                            changed = true;
                        }
                        ui.close();
                    }
                }
            }
            if changed {
                self.config_dirty = true;
            }
        });
        ui.separator();
        let pane = self.pane_mut();
        // The original keeps "Show hidden files" in the View menu (Ctrl+H) and puts
        // "Hidden Files Last" in this one; this build has no hidden-last ordering, so
        // the plain switch stays here rather than being lost. "Show previews" is the
        // same switch as the one in the Pictures submenu below.
        if ui
            .checkbox(&mut pane.show_hidden, "Show hidden files")
            .changed()
        {
            pane.reload();
        }
        ui.separator();
        ui.menu_button("Pictures", |ui| {
            let pane = &mut *pane;
            let mut changed = false;
            if ui
                .checkbox(&mut pane.gallery_names, "File names under pictures")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut pane.gallery_dims, "Size under pictures")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut pane.gallery_round, "Rounded corners")
                .changed()
            {
                changed = true;
            }
            if ui.checkbox(&mut pane.previews, "Show previews").changed() {
                changed = true;
            }
            ui.separator();
            if ui
                .selectable_label(pane.pictures, "Picture wall (F4)")
                .clicked()
            {
                let on = !pane.pictures;
                pane.set_pictures(on);
                changed = true;
                ui.close();
            }
            if changed {}
        });
    }

    fn tab_strip(&mut self, ctx: &egui::Context) {
        let mut to_activate: Option<usize> = None;
        let mut to_close: Option<usize> = None;
        let mut add_tab = false;
        egui::TopBottomPanel::top("tabs")
            .exact_height(33.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .inner_margin(Margin::symmetric(6, 0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let pane = &tab.panes[tab.active];
                        let title = if pane.is_trash() {
                            "Trash".to_string()
                        } else {
                            pane.path
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| pane.display_path())
                        };
                        let font = FontId::proportional(12.5);
                        let text_w = ui
                            .fonts_mut(|f| f.layout_no_wrap(title.clone(), font.clone(), TEXT))
                            .size()
                            .x;
                        let w = (text_w + 62.0).clamp(104.0, 230.0);
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
                        let is_active = i == self.active_tab;
                        let tab_bg = Rect::from_min_max(
                            Pos2::new(rect.left(), rect.top() + 3.0),
                            Pos2::new(rect.right(), rect.bottom()),
                        );
                        if is_active {
                            ui.painter().rect_filled(
                                tab_bg,
                                CornerRadius {
                                    nw: 4,
                                    ne: 4,
                                    sw: 0,
                                    se: 0,
                                },
                                HEADER_BG,
                            );
                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    Pos2::new(rect.left(), rect.bottom() - 2.0),
                                    Pos2::new(rect.right(), rect.bottom()),
                                ),
                                0.0,
                                ACCENT,
                            );
                        } else if resp.hovered() {
                            ui.painter().rect_filled(
                                tab_bg,
                                CornerRadius {
                                    nw: 4,
                                    ne: 4,
                                    sw: 0,
                                    se: 0,
                                },
                                Color32::from_rgb(50, 55, 60),
                            );
                        }
                        let icon_rect = Rect::from_center_size(
                            Pos2::new(rect.left() + 16.0, rect.center().y),
                            Vec2::splat(15.0),
                        );
                        theme::draw_kind_icon(ui.painter(), icon_rect, fsutil::Kind::Folder, false);
                        let label = theme::elide(ui, &title, w - 52.0, font.clone());
                        ui.painter().text(
                            Pos2::new(rect.left() + 27.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            font,
                            if is_active { TEXT } else { TEXT_DIM },
                        );
                        let close_rect = Rect::from_center_size(
                            Pos2::new(rect.right() - 14.0, rect.center().y),
                            Vec2::splat(16.0),
                        );
                        let close_resp =
                            ui.interact(close_rect, Id::new(("tab_close", i)), Sense::click());
                        if close_resp.hovered() {
                            ui.painter().rect_filled(
                                close_rect,
                                CornerRadius::same(3),
                                ROW_HOVER,
                            );
                        }
                        theme::draw_glyph(
                            ui.painter(),
                            close_rect,
                            "close",
                            if close_resp.hovered() { TEXT } else { TEXT_DIM },
                        );
                        if close_resp.clicked() {
                            to_close = Some(i);
                        } else if resp.clicked() {
                            to_activate = Some(i);
                        }
                        if resp.middle_clicked() {
                            to_close = Some(i);
                        }
                    }
                    let (add_rect, add_resp) =
                        ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
                    if add_resp.hovered() {
                        ui.painter()
                            .rect_filled(add_rect, CornerRadius::same(3), ROW_HOVER);
                    }
                    theme::draw_glyph(ui.painter(), add_rect, "plus", TEXT_DIM);
                    if add_resp.clicked() {
                        add_tab = true;
                    }
                });
            });
        if let Some(i) = to_activate {
            self.active_tab = i;
        }
        if let Some(i) = to_close {
            self.close_tab(i);
        }
        if add_tab {
            let dir = self.pane().current_dir();
            self.new_tab_public(dir);
        }
    }

    fn places_panel(&mut self, ctx: &egui::Context) {
        if !self.places_visible {
            return;
        }
        let mut navigate: Option<PathBuf> = None;
        let mut new_tab_path: Option<PathBuf> = None;
        let mut remove_bookmark: Option<PathBuf> = None;
        let mut rename_bookmark: Option<(PathBuf, String)> = None;
        let mut move_bookmark: Option<(usize, i32)> = None;
        let mut add_shortcut = false;
        // Added feature: rows of the two recent groups. A click on a file row
        // opens the file itself (that is the point of a recent list); a click
        // on a folder row walks there. Both menus can also open the file's
        // folder with the file marked, which is what the original's places
        // panel offers for a recently used document.
        let mut open_recent_file: Option<PathBuf> = None;
        let mut show_recent_in_folder: Option<PathBuf> = None;
        let mut copy_recent_path: Option<PathBuf> = None;
        let mut forget_location: Option<PathBuf> = None;
        let mut fold_section = false;
        let mut open_dups = false;
        let mut open_bulk = false;
        let mut open_quick = false;
        // Drag-and-drop state for the shortcut rows. `start_drag` and
        // `finish_drag` are this frame's edges; `drop_slot` is the gap the
        // pointer is over right now, counted among the person's own shortcuts.
        let mut start_drag: Option<usize> = None;
        let mut finish_drag = false;
        let mut drop_slot: Option<usize> = None;
        let mut colour_pick: Option<(PathBuf, Option<[u8; 3]>)> = None;
        let current = self.pane().current_dir();
        let is_trash = self.pane().is_trash();
        // The navigation panel lists every place the person can jump to: the
        // standard folders first, then the shortcuts they pinned themselves.
        let rows = self.shortcut_entries();
        let standard = rows.len() - self.config.bookmarks.len();
        let keys = self.config.shortcut_keys;
        let expanded = self.config.shortcuts_expanded;
        let carrying = self.shortcut_drag;

        egui::SidePanel::left("places")
            .resizable(true)
            .default_width(208.0)
            .width_range(150.0..=340.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .inner_margin(Margin::symmetric(6, 8)),
            )
            .show(ctx, |ui| {
                // The panel now carries four groups plus whatever the person has
                // pinned, so it scrolls when the window is short instead of
                // pushing the last group off the bottom edge (the original's
                // places panel scrolls for the same reason).
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let (fold, add) = shortcuts_header(ui, expanded, self.config.bookmarks.len());
                if fold {
                    fold_section = true;
                }
                if add {
                    add_shortcut = true;
                }
                if expanded {
                    // Where each of the person's own rows sits this frame, so a
                    // drop can be turned into the gap under the pointer.
                    let mut pinned_rects: Vec<(usize, Rect)> = Vec::new();
                    for (i, (name, path, icon)) in rows.iter().enumerate() {
                        let on_trash = path.to_string_lossy() == TRASH_URI;
                        let selected = if on_trash {
                            is_trash
                        } else {
                            !is_trash && current == *path
                        };
                        let hint = if keys && i < 9 {
                            Some(format!("Alt+{}", i + 1))
                        } else {
                            None
                        };
                        let colour = self.config.shortcut_color(path);
                        let mut resp =
                            shortcut_row(ui, name, *icon, selected, hint.as_deref(), colour);
                        let tip = match &hint {
                            Some(k) => format!("{}\n{k}", path.display()),
                            None => path.display().to_string(),
                        };
                        resp = resp.on_hover_text(tip);
                        if resp.clicked() {
                            navigate = Some(path.clone());
                        }
                        if resp.middle_clicked() {
                            new_tab_path = Some(path.clone());
                        }
                        if i >= standard {
                            let bm_index = i - standard;
                            pinned_rects.push((bm_index, resp.rect));
                            if resp.drag_started() {
                                start_drag = Some(bm_index);
                            }
                            if carrying == Some(bm_index) || start_drag == Some(bm_index) {
                                // The row being carried keeps its place but is
                                // outlined, so the drop line is easy to read.
                                ui.painter().rect_stroke(
                                    resp.rect.shrink2(Vec2::new(1.0, 1.0)),
                                    CornerRadius::same(3),
                                    Stroke::new(1.0, ACCENT),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
                            let bm_path = path.clone();
                            let bm_name = name.clone();
                            let pick_path = path.clone();
                            resp.context_menu(|ui| {
                                ui.set_min_width(210.0);
                                if menu_row(ui, "Open in New Tab", "").clicked() {
                                    new_tab_path = Some(bm_path.clone());
                                    ui.close();
                                }
                                if menu_row(ui, "Rename Shortcut…", "").clicked() {
                                    rename_bookmark = Some((bm_path.clone(), bm_name.clone()));
                                    ui.close();
                                }
                                if bm_index > 0 && menu_row(ui, "Move Up", "").clicked() {
                                    move_bookmark = Some((bm_index, -1));
                                    ui.close();
                                }
                                if menu_row(ui, "Move Down", "").clicked() {
                                    move_bookmark = Some((bm_index, 1));
                                    ui.close();
                                }
                                colour_menu_item(ui, "Shortcut Colour", colour, &mut colour_pick, &pick_path);
                                ui.separator();
                                if menu_row(ui, "Remove Shortcut", "").clicked() {
                                    remove_bookmark = Some(bm_path.clone());
                                    ui.close();
                                }
                            });
                        } else {
                            // The standard rows are shortcuts too, so they take a
                            // colour the same way; they just cannot be moved or
                            // removed, because the original's own list fixes them.
                            let pick_path = path.clone();
                            resp.context_menu(|ui| {
                                ui.set_min_width(200.0);
                                if menu_row(ui, "Open in New Tab", "").clicked() {
                                    new_tab_path = Some(pick_path.clone());
                                    ui.close();
                                }
                                colour_menu_item(ui, "Shortcut Colour", colour, &mut colour_pick, &pick_path);
                            });
                        }
                    }

                    // The gap the pointer is over. Half a row decides whether the
                    // line sits above or below the row underneath the pointer; a
                    // pointer carried off the panel sideways drops nothing, which
                    // is how the original's view refuses a drop outside itself.
                    let active = carrying.or(start_drag);
                    if active.is_some() {
                        if let (Some(p), false) = (ctx.pointer_latest_pos(), pinned_rects.is_empty()) {
                            let first = pinned_rects[0].1;
                            if p.x >= first.left() - 6.0 && p.x <= first.right() + 6.0 {
                                let mut slot = pinned_rects.len();
                                for (k, (_, rect)) in pinned_rects.iter().enumerate() {
                                    if p.y < rect.center().y {
                                        slot = k;
                                        break;
                                    }
                                }
                                drop_slot = Some(slot);
                                let y = if slot < pinned_rects.len() {
                                    pinned_rects[slot].1.top()
                                } else {
                                    pinned_rects[pinned_rects.len() - 1].1.bottom()
                                };
                                let x0 = first.left() + 4.0;
                                let x1 = first.right() - 4.0;
                                ui.painter().line_segment(
                                    [Pos2::new(x0, y), Pos2::new(x1, y)],
                                    Stroke::new(2.0, ACCENT),
                                );
                                ui.painter()
                                    .circle_filled(Pos2::new(x0, y), 3.0, ACCENT);
                            }
                        }
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        // A release outside the window never reaches a row, so
                        // "no button held any more" ends the drag as well.
                        if !ctx.input(|i| i.pointer.any_down()) {
                            finish_drag = true;
                        }
                    }

                    if add_row(ui, "Add Shortcut") {
                        add_shortcut = true;
                    }
                }

                // The original's *Remote* group, above Recent and Devices: the
                // network folders this desktop already has. It carries what is
                // mounted rather than a way to mount something, so the row is a
                // jump, not a connection dialog.
                let network = fsutil::network_places();
                ui.add_space(8.0);
                if recent_group_header(
                    ui,
                    "Network",
                    self.config.network_expanded,
                    network.len(),
                ) {
                    self.config.network_expanded = !self.config.network_expanded;
                    self.config_dirty = true;
                }
                if self.config.network_expanded {
                if network.is_empty() {
                    // The original's own sentence for a Remote group with
                    // nothing in it (0x1bf952 in libdolphinprivate.so.6).
                    quiet_row(ui, "No shared folders found");
                } else {
                    for (label, path) in network.iter() {
                        let selected = !is_trash && current == *path;
                        let resp = place_row(ui, label, theme::PlaceIcon::Network, selected)
                            .on_hover_text(path.display().to_string());
                        if resp.clicked() {
                            navigate = Some(path.clone());
                        }
                    }
                }
                }

                // Added feature: the two groups the original's navigation panel
                // shows under Places — *Recent Files* (0x1bf937's "No recently
                // used items" is the original's own wording for the empty case,
                // next to 0x1bf92a's worker name `recentlyused` in
                // libdolphinprivate.so.6)
                // and *Recent Locations*. Dolphin leaves both empty until an
                // application publishes places into them; here they carry the
                // desktop's recently used files and the folders this window has
                // walked through, so the two headings are worth their space.
                self.refresh_recent_files();
                let recent_files = self.recent_files.clone();
                ui.add_space(8.0);
                if recent_group_header(
                    ui,
                    "Recent Files",
                    self.config.recent_files_expanded,
                    recent_files.len(),
                ) {
                    self.config.recent_files_expanded = !self.config.recent_files_expanded;
                    self.config_dirty = true;
                }
                if self.config.recent_files_expanded {
                    if recent_files.is_empty() {
                        quiet_row(ui, "No recently used items");
                    } else {
                        for item in recent_files.iter() {
                            let resp = row_impl(
                                ui,
                                &item.name,
                                Some(theme::PlaceIcon::File),
                                false,
                                None,
                            )
                            .on_hover_text(format!(
                                "{}\n{}",
                                item.path.display(),
                                fsutil::human_date(item.modified)
                            ));
                            if resp.clicked() {
                                open_recent_file = Some(item.path.clone());
                            }
                            let path = item.path.clone();
                            let name = item.name.clone();
                            let dir = path.parent().map(|p| p.to_path_buf());
                            resp.context_menu(|ui| {
                                ui.set_min_width(214.0);
                                if menu_row(ui, "Open", "").clicked() {
                                    open_recent_file = Some(path.clone());
                                    ui.close();
                                }
                                if menu_row(ui, "Show in Folder", "").clicked() {
                                    show_recent_in_folder = Some(path.clone());
                                    ui.close();
                                }
                                if menu_row(ui, "Open Folder in New Tab", "").clicked() {
                                    if let Some(dir) = &dir {
                                        new_tab_path = Some(dir.clone());
                                    }
                                    ui.close();
                                }
                                ui.separator();
                                if menu_row(ui, "Copy File Name", "").clicked() {
                                    copy_recent_path = Some(PathBuf::from(name.clone()));
                                    ui.close();
                                }
                                if menu_row(ui, "Copy Full Path", "").clicked() {
                                    copy_recent_path = Some(path.clone());
                                    ui.close();
                                }
                            });
                        }
                    }
                }

                let locations = self.config.recent_locations.clone();
                ui.add_space(6.0);
                if recent_group_header(
                    ui,
                    "Recent Locations",
                    self.config.recent_locations_expanded,
                    locations.len(),
                ) {
                    self.config.recent_locations_expanded = !self.config.recent_locations_expanded;
                    self.config_dirty = true;
                }
                if self.config.recent_locations_expanded {
                    if locations.is_empty() {
                        quiet_row(ui, "No places visited yet");
                    } else {
                        for folder in locations.iter() {
                            let label = folder
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "/".to_string());
                            let selected = !is_trash && current == *folder;
                            let resp = row_impl(
                                ui,
                                &label,
                                Some(theme::PlaceIcon::Recent),
                                selected,
                                None,
                            )
                            .on_hover_text(folder.display().to_string());
                            if resp.clicked() {
                                navigate = Some(folder.clone());
                            }
                            if resp.middle_clicked() {
                                new_tab_path = Some(folder.clone());
                            }
                            let folder = folder.clone();
                            resp.context_menu(|ui| {
                                ui.set_min_width(210.0);
                                if menu_row(ui, "Open in New Tab", "").clicked() {
                                    new_tab_path = Some(folder.clone());
                                    ui.close();
                                }
                                if menu_row(ui, "Copy Full Path", "").clicked() {
                                    copy_recent_path = Some(folder.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if menu_row(ui, "Forget This Location", "").clicked() {
                                    forget_location = Some(folder.clone());
                                    ui.close();
                                }
                            });
                        }
                    }
                }

                ui.add_space(10.0);
                section_header(ui, "Devices");
                for (label, path) in fsutil::mounted_devices() {
                    let selected = !is_trash && current == path;
                    let free = fsutil::free_space(&path)
                        .map(|(f, t)| {
                            format!(
                                "{} free of {}",
                                fsutil::format_size(f),
                                fsutil::format_size(t)
                            )
                        })
                        .unwrap_or_default();
                    let resp = place_row(ui, &label, theme::PlaceIcon::Disk, selected)
                        .on_hover_text(free);
                    if resp.clicked() {
                        navigate = Some(path.clone());
                    }
                }

                ui.add_space(12.0);
                section_header(ui, "Added tools");
                if tool_row(ui, "Duplicates", "Find identical files in this folder (Ctrl+Shift+D)", Some("Ctrl+Shift+D")) {
                    open_dups = true;
                }
                if tool_row(ui, "Bulk Rename", "Rename many files with a pattern (Ctrl+Shift+R)", Some("Ctrl+Shift+R")) {
                    open_bulk = true;
                }
                if tool_row(ui, "Quick Look", "Preview the selected file (Space)", Some("Space")) {
                    open_quick = true;
                }
                // A little air under the last row, so a section that only just
                // fits is not cut in half by the panel's own bottom edge.
                ui.add_space(10.0);
                    });
            });

        if let Some(p) = navigate {
            self.pane_mut().navigate(p);
        }
        if let Some(path) = open_recent_file {
            if path.is_file() {
                self.reqs.push(Request::OpenPath(path));
            } else {
                self.toast("That file is no longer there.");
                self.recent_files_at = None;
            }
        }
        if let Some(path) = show_recent_in_folder {
            // The original's places panel opens the item's folder with the item
            // marked; the pane's pending-select does exactly that once the
            // listing has arrived.
            if let Some(dir) = path.parent().map(|p| p.to_path_buf()) {
                self.pane_mut().navigate(dir);
                if let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) {
                    self.pane_mut().pending_select = Some(name);
                    self.pane_mut().apply_pending_select();
                }
            }
        }
        if let Some(path) = copy_recent_path {
            let text = path.display().to_string();
            ctx.copy_text(text.clone());
            self.toast(&format!("Copied {text}"));
        }
        if let Some(path) = forget_location {
            self.config.recent_locations.retain(|p| p != &path);
            self.config_dirty = true;
        }
        if let Some(p) = new_tab_path {
            self.new_tab_public(p);
        }
        if fold_section {
            self.config.shortcuts_expanded = !self.config.shortcuts_expanded;
            self.config_dirty = true;
        }
        if add_shortcut {
            let dir = self.pane().current_dir();
            self.open_shortcut_editor(None, dir);
        }
        if let Some((path, name)) = rename_bookmark {
            self.open_shortcut_editor(Some(path.clone()), path);
            if let Some(edit) = self.shortcut_edit.as_mut() {
                edit.name = name;
            }
        }
        if let Some((idx, delta)) = move_bookmark {
            let n = self.config.bookmarks.len();
            let to = idx as i32 + delta;
            if to >= 0 && (to as usize) < n {
                // Dropping a row one place down is the same as moving it to the
                // gap after its neighbour, which is what the model's own move
                // takes: an insertion slot, not a destination row.
                let slot = if delta > 0 { to as usize + 1 } else { to as usize };
                if config::move_shortcut(&mut self.config.bookmarks, idx, slot) {
                    self.config_dirty = true;
                }
            }
        }
        // A row that was let go: the gap under the pointer becomes its new place.
        if start_drag.is_some() {
            self.shortcut_drag = start_drag;
        }
        if finish_drag {
            if let (Some(from), Some(to)) = (self.shortcut_drag, drop_slot) {
                if config::move_shortcut(&mut self.config.bookmarks, from, to) {
                    self.config_dirty = true;
                    self.toast("Shortcut moved");
                }
            }
            self.shortcut_drag = None;
        }
        if let Some((path, rgb)) = colour_pick {
            self.set_shortcut_color(&path, rgb);
        }
        if let Some(p) = remove_bookmark {
            self.config.bookmarks.retain(|b| b.path != p);
            self.config.shortcut_colors.retain(|c| c.path != p);
            self.config_dirty = true;
            self.toast("Shortcut removed");
        }
        if open_dups {
            let root = self.pane().current_dir();
            self.dups = Some(Dups {
                scan: DupScan::start(root),
            });
        }
        if open_bulk {
            let paths = self.pane().selected_paths();
            if paths.is_empty() {
                self.toast("Select the files to rename first.");
            } else {
                self.bulk = Some(Bulk {
                    paths,
                    pattern: "{name}{ext}".to_string(),
                    start: 1,
                    error: None,
                });
            }
        }
        if open_quick {
            if let Some(entry) = self.pane().primary_selection().cloned() {
                self.reqs.push(Request::QuickLook(entry.path));
            }
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(TOOLBAR_BG)
                    .inner_margin(Margin::symmetric(10, 3)),
            )
            .show(ctx, |ui| {
                // The bar's own right-click menu. The original has exactly one
                // entry there: `Show zoom slider` (`FUN_001ec540` @0x1ec540 reads
                // the `GeneralSettings` key `ShowZoomSlider`, writes it back and
                // calls the two setters that show or hide the slider widgets).
                let strip = ui.max_rect();
                let bar = ui.interact(
                    strip,
                    egui::Id::new("status_bar_menu"),
                    egui::Sense::click(),
                );
                bar.context_menu(|ui| {
                    ui.set_min_width(196.0);
                    let mut on = self.config.show_zoom_slider;
                    if ui.checkbox(&mut on, "Show zoom slider").changed() {
                        self.config.show_zoom_slider = on;
                        self.config_dirty = true;
                    }
                });
                ui.horizontal_centered(|ui| {
                    let pane = self.pane();
                    let sel_count = pane.sel.len();
                    let shown = pane.display_entries();
                    let mut text = if pane.search.is_some() {
                        let s = pane.search.as_ref().unwrap();
                        if s.pictures_below {
                            // The wall's own walk: "found" would say a search
                            // was typed when nothing was.
                            if !s.done {
                                format!("Looking below… {} pictures so far", shown.len())
                            } else {
                                format!(
                                    "{} picture{} below",
                                    shown.len(),
                                    if shown.len() == 1 { "" } else { "s" }
                                )
                            }
                        } else if !s.done {
                            format!("Searching… {} items found", shown.len())
                        } else {
                            format!(
                                "{} item{} found",
                                shown.len(),
                                if shown.len() == 1 { "" } else { "s" }
                            )
                        }
                    } else if pane.is_trash() {
                        format!(
                            "{} item{} in Trash",
                            shown.len(),
                            if shown.len() == 1 { "" } else { "s" }
                        )
                    } else {
                        let folders = shown.iter().filter(|e| e.is_dir).count();
                        let files = shown.len() - folders;
                        let total: u64 =
                            shown.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                        status_counts(folders, files, total)
                    };
                    if sel_count > 0 {
                        let folders = pane
                            .sel
                            .iter()
                            .filter_map(|i| shown.get(*i))
                            .filter(|e| e.is_dir)
                            .count();
                        let files = sel_count - folders;
                        let bytes: u64 = pane
                            .sel
                            .iter()
                            .filter_map(|i| shown.get(*i))
                            .filter(|e| !e.is_dir)
                            .map(|e| e.size)
                            .sum();
                        text = status_selection(folders, files, bytes);
                    }
                    ui.label(egui::RichText::new(text).size(11.5).color(TEXT_DIM));

                    let mut zoom_value = self.pane().zoom_level() as f32;
                    let icon = self.pane().icon_size() as u32;
                    let mut zoom_changed = false;
                    // In the gallery the same − slider + strip changes the
                    // thumbnail size instead, over the browser's 50..400 px range.
                    let pictures = self.pane().pictures;
                    let mut thumb_value = self.pane().thumb_px;

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pictures {
                            if small_button(ui, "+").on_hover_text("Bigger thumbnails").clicked() {
                                self.pane_mut().thumb_by(25.0);
                                self.config.thumb_px = self.pane().thumb_px;
                                self.config_dirty = true;
                            }
                            let slider =
                                theme::strip_slider(ui, &mut thumb_value, 50.0..=400.0, 110.0);
                            if slider.changed() {
                                let px = gallery::clamp_thumb(thumb_value);
                                self.pane_mut().thumb_px = px;
                                self.config.thumb_px = px;
                                self.config_dirty = true;
                            }
                            if small_button(ui, "−").on_hover_text("Smaller thumbnails").clicked() {
                                self.pane_mut().thumb_by(-25.0);
                                self.config.thumb_px = self.pane().thumb_px;
                                self.config_dirty = true;
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} px",
                                    self.pane().thumb_px.round() as u32
                                ))
                                .size(11.0)
                                .color(TEXT_FAINT),
                            );
                            ui.add_space(14.0);
                            if let Some((f, t)) = fsutil::free_space(&self.pane().current_dir()) {
                                if t > 0 {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} free",
                                            fsutil::format_size(f)
                                        ))
                                        .size(11.0)
                                        .color(TEXT_FAINT),
                                    )
                                    .on_hover_text(format!(
                                        "{} free out of {} ({}% used)",
                                        fsutil::format_size(f),
                                        fsutil::format_size(t),
                                        fsutil::used_percent(f, t)
                                    ));
                                }
                            }
                            return;
                        }
                        // `GeneralSettings` `ShowZoomSlider`: the original's
                        // status-bar slider is optional (`FUN_001ec540` @0x1ec540
                        // shows or hides it from the bar's own context menu). When
                        // it is off, only the size readout stays — the same `NN px`
                        // label the slider's tip carries.
                        if self.config.show_zoom_slider {
                            if small_button(ui, "+").on_hover_text("Zoom in").clicked() {
                                self.pane_mut().zoom_by(1);
                                zoom_changed = true;
                            }
                            let slider = theme::strip_slider(
                                ui,
                                &mut zoom_value,
                                0.0..=theme::MAX_ZOOM as f32,
                                110.0,
                            );
                            if slider.changed() {
                                let z = zoom_value.round() as u8;
                                self.pane_mut().set_zoom_level(z);
                                zoom_changed = true;
                            }
                            if small_button(ui, "−").on_hover_text("Zoom out").clicked() {
                                self.pane_mut().zoom_by(-1);
                                zoom_changed = true;
                            }
                        }
                        ui.label(
                            egui::RichText::new(format!("{} px", icon))
                                .size(11.0)
                                .color(TEXT_FAINT),
                        );
                        ui.add_space(14.0);
                        if let Some((f, t)) = fsutil::free_space(&self.pane().current_dir()) {
                            // `FUN_001e6a40` @0x1e6a40 draws the free-disk-space
                            // button: its text is `%1 free` (string 0x12ec43,
                            // KIO::convertSize of the free bytes) and the whole
                            // sentence `%1 free out of %2 (%3% used)`
                            // (0x12ec68 / 0x137448) is only the tooltip. The
                            // widget is hidden when the volume does not report a
                            // size, so a zero total shows nothing at all.
                            if t > 0 {
                                let pct = fsutil::used_percent(f, t);
                                // `FUN_001e6a40` @0x1e6a40 also drives a
                                // `KCapacityBar` that sits next to the free-space
                                // button: `KCapacityBar::setValue(iVar4)` with the
                                // same rounded used percentage the tooltip prints,
                                // inside the same `total != 0` test that hides the
                                // whole widget on a volume of unknown size.
                                let (bar, _) = ui.allocate_exact_size(
                                    Vec2::new(34.0, 6.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    CornerRadius::same(2),
                                    Color32::from_rgb(48, 53, 58),
                                );
                                let filled = (bar.width() * (pct as f32 / 100.0)).max(0.0);
                                if filled > 0.5 {
                                    let fill = Rect::from_min_size(
                                        bar.min,
                                        Vec2::new(filled, bar.height()),
                                    );
                                    ui.painter().rect_filled(
                                        fill,
                                        CornerRadius::same(2),
                                        ACCENT,
                                    );
                                }
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} free",
                                        fsutil::format_size(f)
                                    ))
                                    .size(11.0)
                                    .color(TEXT_FAINT),
                                )
                                .on_hover_text(format!(
                                    "{} free out of {} ({}% used)",
                                    fsutil::format_size(f),
                                    fsutil::format_size(t),
                                    pct
                                ));
                            }
                        }
                    });
                    if zoom_changed {
                        self.config.zoom = self.pane().zoom;
                        self.config.zoom_plain = self.pane().zoom_plain;
                        self.config_dirty = true;
                    }
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(VIEW_BG))
            .show(ctx, |ui| {
                if !self.split || self.tabs[self.active_tab].panes.len() < 2 {
                    let mut reqs = std::mem::take(&mut self.reqs);
                    let tab = &mut self.tabs[self.active_tab];
                    let mut vctx = ViewCtx {
                        thumbs: &mut self.thumbs,
                        clipboard: &self.clipboard,
                        reqs: &mut reqs,
                        split: self.split,
                        active: true,
                        show_location_bar: self.split,
                    };
                    tab.panes[0].ui(ui, &mut vctx);
                    tab.active = 0;
                    self.reqs = reqs;
                    return;
                }
                let rect = ui.available_rect_before_wrap();
                let mid = rect.left() + rect.width() * self.split_frac;
                let left_rect = Rect::from_min_max(
                    rect.min,
                    Pos2::new((mid - 3.0).max(rect.left() + 80.0), rect.bottom()),
                );
                let right_rect = Rect::from_min_max(
                    Pos2::new((mid + 3.0).min(rect.right() - 80.0), rect.top()),
                    rect.max,
                );
                let divider = Rect::from_min_max(
                    Pos2::new(left_rect.right(), rect.top()),
                    Pos2::new(right_rect.left(), rect.bottom()),
                );
                ui.painter().rect_filled(divider, 0.0, BORDER);
                let drag = ui.interact(divider, Id::new("split_divider"), Sense::drag());
                if drag.dragged() {
                    if let Some(pos) = drag.interact_pointer_pos() {
                        let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.2, 0.8);
                        self.split_frac = frac;
                    }
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                let pressed = ui.input(|i| i.pointer.any_pressed());
                let pointer = ui.input(|i| i.pointer.interact_pos());
                let mut clicked_side: Option<usize> = None;
                let active = self.tabs[self.active_tab].active;
                let mut reqs = std::mem::take(&mut self.reqs);
                {
                    let tab = &mut self.tabs[self.active_tab];
                    let (first, rest) = tab.panes.split_at_mut(1);
                    let mut vctx0 = ViewCtx {
                        thumbs: &mut self.thumbs,
                        clipboard: &self.clipboard,
                        reqs: &mut reqs,
                        split: true,
                        active: active == 0,
                        show_location_bar: true,
                    };
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("pane", first[0].id))
                            .max_rect(left_rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    child.set_clip_rect(left_rect);
                    first[0].ui(&mut child, &mut vctx0);

                    let mut vctx1 = ViewCtx {
                        thumbs: &mut self.thumbs,
                        clipboard: &self.clipboard,
                        reqs: &mut reqs,
                        split: true,
                        active: active == 1,
                        show_location_bar: true,
                    };
                    let mut child2 = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("pane", rest[0].id))
                            .max_rect(right_rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    child2.set_clip_rect(right_rect);
                    rest[0].ui(&mut child2, &mut vctx1);

                    if pressed {
                        if let Some(p) = pointer {
                            if left_rect.contains(p) {
                                clicked_side = Some(0);
                            } else if right_rect.contains(p) {
                                clicked_side = Some(1);
                            }
                        }
                    }
                }
                if let Some(side) = clicked_side {
                    self.tabs[self.active_tab].active = side;
                }
                self.reqs = reqs;
            });
    }

    // -----------------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------------

    fn handle_requests(&mut self, ctx: &egui::Context) {
        let reqs = std::mem::take(&mut self.reqs);
        for req in reqs {
            match req {
                Request::Navigate(path) => {
                    let p = if path.as_os_str() == TRASH_URI {
                        PathBuf::from(TRASH_URI)
                    } else {
                        path
                    };
                    self.pane_mut().navigate(p);
                }
                Request::NavigateOther(path) => {
                    let other = self.active_pane_index();
                    let _ = other;
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        let idx = 1 - tab.active;
                        tab.panes[idx].navigate(path);
                    }
                }
                Request::OpenInNewTab(path) => self.new_tab_public(path),
                Request::OpenInSplit(path) => {
                    self.ensure_split();
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        let idx = 1 - tab.active;
                        tab.panes[idx].navigate(path);
                    }
                }
                Request::AddBookmark(name, path) => {
                    // Offer the editor rather than adding silently, so the row
                    // can be given a name the person recognises.
                    let already = self.config.bookmarks.iter().any(|b| b.path == path);
                    self.open_shortcut_editor(None, path);
                    if let Some(edit) = self.shortcut_edit.as_mut() {
                        edit.name = name;
                        if already {
                            edit.error = Some("This folder is already pinned.".to_string());
                        }
                    }
                }
                Request::RemoveBookmark(path) => {
                    self.config.bookmarks.retain(|b| b.path != path);
                    self.config_dirty = true;
                }
                Request::Properties(paths) => {
                    let folder_size = paths
                        .first()
                        .and_then(|p| self.folder_sizes.get(p).copied());
                    self.props = Some(Props {
                        paths,
                        checksum: None,
                        busy: false,
                        rx: None,
                        folder_size,
                    });
                }
                Request::Trash(paths) => self.ask_or_trash(paths),
                Request::EmptyTrash => self.ask_or_empty_trash(),
                Request::ForgetSettings => {
                    // Every choice back to its factory default, extra tabs
                    // forgotten, and the file written at once so the next start
                    // is a plain window. The folder being shown stays open: the
                    // person asked to forget their settings, not where they were.
                    let dir = self.pane().current_dir();
                    self.config = Config::default();
                    self.places_visible = true;
                    self.info_panel = false;
                    self.unsplit();
                    apply_defaults(self.pane_mut());
                    self.tabs.truncate(1);
                    self.active_tab = 0;
                    self.split_frac = 0.5;
                    self.pane_mut().load(dir);
                    self.persist(Some(ctx));
                    self.toast("Settings are back to their defaults; this window stays open.");
                }
                Request::Restore(paths) => {
                    let mut ok = 0;
                    let mut errors = Vec::new();
                    for p in &paths {
                        let entry = fsutil::list_trash().into_iter().find(|e| &e.path == p);
                        match entry {
                            Some(e) => {
                                if let Err(err) = fsutil::restore_trash_entry(&e) {
                                    errors.push(err);
                                } else {
                                    ok += 1;
                                }
                            }
                            None => errors
                                .push(format!("{} is no longer in the Trash", p.display())),
                        }
                    }
                    if errors.is_empty() {
                        self.toast(&format!(
                            "Restored {} item{}",
                            ok,
                            if ok == 1 { "" } else { "s" }
                        ));
                    } else {
                        self.toast(&errors.join("; "));
                    }
                    self.pane_mut().reload();
                }
                Request::DeletePermanent(paths) => self.ask_or_delete(paths),
                Request::BulkRename(paths) => {
                    self.bulk = Some(Bulk {
                        paths,
                        pattern: "{name}{ext}".to_string(),
                        start: 1,
                        error: None,
                    });
                }
                Request::QuickLook(path) => {
                    let entry = fsutil::Entry::from_path(&path);
                    let content = if entry.is_text() {
                        search::read_text_preview(&path, 200_000)
                    } else {
                        None
                    };
                    self.quick = Some(Quick { path, text: content });
                }
                Request::NewFolder(_dir) => {
                    self.new_folder = Some(NewFolder {
                        name: "New Folder".to_string(),
                        focus: false,
                    });
                }
                Request::Rename(path, new_name) => {
                    let target = path.with_file_name(&new_name);
                    if target.exists() {
                        self.toast("An item with that name already exists.");
                    } else {
                        match std::fs::rename(&path, &target) {
                            Ok(()) => {
                                self.pane_mut().reload();
                                if let Some(idx) =
                                    self.pane().entries.iter().position(|e| e.path == target)
                                {
                                    self.pane_mut().select_single(idx);
                                    self.pane_mut().scroll_to = Some(idx);
                                }
                            }
                            Err(e) => self.toast(&format!("Could not rename: {}", e)),
                        }
                    }
                }
                Request::OpenTerminal(dir) => fsutil::open_terminal(&dir),
                Request::CopyToOther(cut) => {
                    let _ = cut;
                }
                Request::Duplicate(paths) => {
                    for src in &paths {
                        let stem = src
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "item".to_string());
                        let ext = src
                            .extension()
                            .map(|s| format!(".{}", s.to_string_lossy()))
                            .unwrap_or_default();
                        let mut n = 1;
                        let mut target = src.with_file_name(format!("{} copy{}", stem, ext));
                        while target.exists() {
                            target = src.with_file_name(format!("{} copy {}{}", stem, n, ext));
                            n += 1;
                        }
                        let res = if src.is_dir() {
                            copy_dir(src, &target)
                        } else {
                            std::fs::copy(src, &target)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        };
                        if let Err(e) = res {
                            self.toast(&format!("Could not duplicate: {}", e));
                        }
                    }
                    self.pane_mut().reload();
                }
                Request::FolderSize(paths) => {
                    // `ContentDisplaySettings::RecursiveDirectorySizeLimit`:
                    // "Show size of contents, up to N folders", factory 10.
                    let depth = self.config.size_limit.clamp(
                        search::SIZE_LIMIT_MIN,
                        search::SIZE_LIMIT_MAX,
                    );
                    for p in paths {
                        let tx = self.size_tx.clone();
                        std::thread::spawn(move || {
                            let size = search::dir_size_limited(&p, Some(depth));
                            let _ = tx.send((p, size));
                        });
                    }
                    self.toast("Calculating folder size…");
                }
                Request::FolderColor(paths, rgb) => {
                    let mut changed = 0usize;
                    let mut failure: Option<String> = None;
                    for p in &paths {
                        match foldercolor::write(p, rgb) {
                            Ok(()) => {
                                changed += 1;
                                for tab in &mut self.tabs {
                                    for pane in &mut tab.panes {
                                        for entry in pane.entries.iter_mut() {
                                            if entry.path == *p {
                                                entry.tint = rgb;
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => failure = Some(e),
                        }
                    }
                    if let Some(e) = failure {
                        self.toast(&e);
                    } else if changed == 1 {
                        let what = match rgb.and_then(foldercolor::name_of) {
                            Some(name) => format!("Folder colour: {}", name),
                            None => "Folder colour: normal".to_string(),
                        };
                        self.toast(&what);
                    } else if changed > 1 {
                        let what = match (rgb.and_then(foldercolor::name_of), paths.len()) {
                            (Some(name), n) => format!("{} folders are now {}", n, name),
                            (None, n) => format!("{} folders are back to normal", n),
                        };
                        self.toast(&what);
                    }
                }
                Request::OpenPath(path) => {
                    if path.is_dir() {
                        self.pane_mut().navigate(path);
                    } else {
                        fsutil::open_with_default(&path);
                    }
                }
                Request::OpenWith {
                    id,
                    paths,
                    remember,
                } => {
                    match crate::apps::find(&id) {
                        Some(app) => {
                            if let Err(e) = crate::apps::launch(&app, &paths) {
                                self.toast(&e);
                            } else {
                                let what = if paths.len() == 1 {
                                    format!("Opened in {}", app.name)
                                } else {
                                    format!("{} items opened in {}", paths.len(), app.name)
                                };
                                self.toast(&what);
                            }
                            // The tick box in the chooser: the same file the
                            // desktop itself reads, so the first row of the menu
                            // names this program from now on.
                            if let Some(mime) = remember {
                                match crate::apps::remember(&mime, &id) {
                                    Ok(_) => self.toast(&format!(
                                        "{} now opens {}",
                                        app.name, mime
                                    )),
                                    Err(e) => self.toast(&format!(
                                        "Could not remember that choice: {}",
                                        e
                                    )),
                                }
                            }
                        }
                        None => self.toast("That program is no longer installed."),
                    }
                }
                Request::OpenWithChoose(paths) => {
                    if !paths.is_empty() {
                        let mime = crate::apps::mime_of_selection(&paths);
                        // The list starts on the desktop's own preferred program
                        // for this type, the one the menu's first row names, so
                        // the chooser opens where the person expects; with no
                        // preference the first row is the first program that
                        // claims the type.
                        let wanted = crate::apps::default_for(&mime);
                        let chosen = wanted
                            .and_then(|id| crate::apps::chooser(&mime).iter().position(|a| a.id == id))
                            .unwrap_or(0);
                        self.open_with = Some(OpenWith {
                            mime,
                            paths,
                            filter: String::new(),
                            remember: false,
                            chosen,
                            focus: false,
                        });
                    }
                }
                Request::ReopenClosedTab(back) => {
                    let n = back.min(self.closed_tabs.len().saturating_sub(1));
                    if self.closed_tabs.is_empty() {
                        self.toast("No closed tabs to bring back.");
                    } else {
                        let idx = self.closed_tabs.len() - 1 - n;
                        let path = self.closed_tabs.remove(idx);
                        self.new_tab_public(path);
                        self.toast("Tab brought back.");
                    }
                }
                Request::Cut => {
                    let paths = self.pane().selected_paths();
                    if !paths.is_empty() {
                        self.clipboard.paths = paths;
                        self.clipboard.cut = true;
                        self.toast("Cut to clipboard");
                    }
                }
                Request::Copy => {
                    let paths = self.pane().selected_paths();
                    if !paths.is_empty() {
                        self.clipboard.paths = paths;
                        self.clipboard.cut = false;
                        self.toast("Copied to clipboard");
                    }
                }
                Request::Paste => {
                    let dest = self.pane().current_dir();
                    let mut problems = Vec::new();
                    let mut done = 0;
                    for src in self.clipboard.paths.clone() {
                        let name = src
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let mut target = dest.join(&name);
                        let mut n = 1;
                        while target.exists() {
                            let stem = src
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| name.clone());
                            let ext = src
                                .extension()
                                .map(|s| format!(".{}", s.to_string_lossy()))
                                .unwrap_or_default();
                            target = dest.join(format!("{} {}{}", stem, n, ext));
                            n += 1;
                        }
                        let res = if self.clipboard.cut {
                            std::fs::rename(&src, &target).map_err(|e| e.to_string())
                        } else if src.is_dir() {
                            copy_dir(&src, &target)
                        } else {
                            std::fs::copy(&src, &target)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        };
                        match res {
                            Ok(()) => done += 1,
                            Err(e) => problems.push(e),
                        }
                    }
                    if self.clipboard.cut {
                        self.clipboard.paths.clear();
                    }
                    if problems.is_empty() {
                        self.toast(&format!(
                            "{} item{} pasted",
                            done,
                            if done == 1 { "" } else { "s" }
                        ));
                    } else {
                        self.toast(&problems.join("; "));
                    }
                    self.pane_mut().reload();
                }
                Request::BeginRename(idx) => {
                    if let Some(entry) = self.pane().display_entries().get(idx).cloned() {
                        let base = if entry.is_dir {
                            entry.name.clone()
                        } else {
                            entry
                                .path
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| entry.name.clone())
                        };
                        self.pane_mut().rename = Some(state::RenameState {
                            index: idx,
                            text: base,
                            focus: false,
                        });
                    }
                }
                Request::ZoomChanged => {
                    let zoom = self.pane().zoom;
                    self.config.zoom = zoom;
                    self.config.zoom_plain = self.pane().zoom_plain;
                    self.config_dirty = true;
                }
                Request::OpenViewer(path, play) => {
                    self.open_viewer(&path, play);
                }
                Request::GalleryChanged => {
                    self.config.thumb_px = self.pane().thumb_px;
                    self.config.gallery_names = self.pane().gallery_names;
                    self.config.gallery_dims = self.pane().gallery_dims;
                    self.config.gallery_round = self.pane().gallery_round;
                    self.config.previews = self.pane().previews;
                    self.config_dirty = true;
                }
                Request::Toast(msg) => {
                    if let Some(partner) = msg.strip_prefix("__compare__") {
                        if let Some(viewer) = self.viewer.as_mut() {
                            if viewer.start_compare() {
                                if let Some(cmp) = viewer.compare.as_mut() {
                                    cmp.b = PathBuf::from(partner);
                                }
                            }
                        }
                    } else {
                        self.toast(&msg)
                    }
                }
            }
        }
        self.poll_background();
        let _ = ctx;
    }

    fn poll_background(&mut self) {
        while let Ok((path, size)) = self.size_rx.try_recv() {
            self.folder_sizes.insert(path, size);
        }
        if let Some(props) = self.props.as_mut() {
            if let Some(rx) = &props.rx {
                if let Ok(result) = rx.try_recv() {
                    props.busy = false;
                    props.checksum = result;
                    props.rx = None;
                } else {
                    props.busy = true;
                }
            }
        }
        let mut done_any = false;
        for tab in self.tabs.iter_mut() {
            for pane in tab.panes.iter_mut() {
                let mut finished = false;
                if let Some(s) = pane.search.as_mut() {
                    s.poll();
                    finished = s.done;
                }
                if finished {
                    done_any = true;
                }
            }
        }
        if done_any {
            for tab in self.tabs.iter_mut() {
                for pane in tab.panes.iter_mut() {
                    if pane.search.as_ref().map(|s| s.done).unwrap_or(false) {
                        let entries = pane.search.as_ref().map(|s| s.results.clone());
                        if let Some(entries) = entries {
                            let mut sorted = entries;
                            sort_search_results(&mut sorted, pane.sort, pane.asc);
                            if let Some(s) = pane.search.as_mut() {
                                s.results = sorted;
                            }
                        }
                    }
                }
            }
        }
        if let Some(dups) = self.dups.as_mut() {
            dups.scan.poll();
        }
    }

    pub fn toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
    }

    // -----------------------------------------------------------------------
    // Deleting
    // -----------------------------------------------------------------------

    /// Delete (Del) on a marked selection. The original stops here with a modal
    /// ("Move %2 to the Trash", `ConfirmTrash` in its Confirmations page); the
    /// person asked for the mark and the key press to be the whole action, so the
    /// dialog only appears when that switch is on, and it ships off.
    ///
    /// The wording below is the original's own, read out of
    /// `KIO::JobUiDelegate::askDeleteConfirmation` @0x1a8a20 in
    /// `libKF6KIOWidgets.so.6.31.0`: the continue button says "Move to Trash",
    /// and the question is "Do you really want to move this item to the Trash?"
    /// with the item's name under it (a list when several are marked). See
    /// MECHANICS.md, "The delete question".
    pub fn ask_or_trash(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        // Inside the Trash there is nowhere further to move an item to, so Delete
        // there means what it means in the original: gone for good. Measured in
        // `KIO::DeleteOrTrashJob::DeleteOrTrashJob` @0x1b0fa0, which rewrites a
        // Trash request to a Delete request when the URL's scheme is "trash".
        if self.pane().is_trash() {
            self.ask_or_delete(paths);
            return;
        }
        if self.config.confirm_trash {
            let name = display_name(&paths[0]);
            let body = if paths.len() == 1 {
                format!("Do you really want to move this item to the Trash?\n\n{name}")
            } else {
                format!(
                    "Do you really want to move these items to the Trash?\n\n{}",
                    name_list(&paths)
                )
            };
            self.confirm = Some(Confirm {
                title: "Move to Trash".to_string(),
                body,
                action: ConfirmAction::Trash(paths),
            });
            return;
        }
        self.trash_now(paths);
    }

    /// Shift+Delete on a marked selection. Unlike the Trash move, the original
    /// *does* ask here by default: `ConfirmDelete` is read with a default of
    /// true in `KIO::WidgetsAskUserActionHandler::askUserDelete` @0x1a7980
    /// (`libKF6KIOWidgets.so.6.31.0`, key at 0x21e8d4), and the question is
    /// "Do you really want to permanently delete this item?" followed by
    /// "This action cannot be undone." (`askDeleteConfirmation` @0x1a8a20).
    pub fn ask_or_delete(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.config.confirm_delete {
            let name = display_name(&paths[0]);
            let body = if paths.len() == 1 {
                format!(
                    "Do you really want to permanently delete this item?\n\n{name}\n\nThis action cannot be undone."
                )
            } else {
                format!(
                    "Do you really want to permanently delete these items?\n\n{}\n\nThis action cannot be undone.",
                    name_list(&paths)
                )
            };
            self.confirm = Some(Confirm {
                title: "Delete Permanently".to_string(),
                body,
                action: ConfirmAction::Delete(paths),
            });
            return;
        }
        self.delete_now(paths);
    }

    /// Move to the Trash, say how many went, and keep the way back for a few
    /// seconds. The wording is the original's own (`Trash operation completed.`),
    /// with the count that was already in this build's toast.
    pub fn trash_now(&mut self, paths: Vec<PathBuf>) {
        let (moved, records, problem) = fsutil::trash_with_records(&paths);
        if moved == 0 {
            self.toast(&problem.unwrap_or_else(|| "Nothing was moved.".to_string()));
            return;
        }
        let msg = format!(
            "Trash operation completed. {moved} item{} moved.",
            if moved == 1 { "" } else { "s" }
        );
        self.toast(&match problem {
            Some(p) => format!("{msg} {p}"),
            None => msg,
        });
        self.undo_trash = Some(UndoTrash {
            records,
            at: Instant::now(),
        });
        self.reload_after_delete();
    }

    /// Delete for good. `Delete operation completed.` is the original's wording
    /// for the same action, so the little toast reads the same way.
    pub fn delete_now(&mut self, paths: Vec<PathBuf>) {
        let (gone, problems) = fsutil::delete_permanently(&paths);
        // A permanent delete ends the offer: what went cannot come back, and an
        // older offer would put something else back under a misleading toast.
        self.undo_trash = None;
        if problems.is_empty() {
            if gone > 0 {
                self.toast("Delete operation completed.");
                self.reload_after_delete();
            }
        } else {
            self.toast(&format!("Could not delete: {}", problems.join(", ")));
            if gone > 0 {
                self.reload_after_delete();
            }
        }
    }

    /// Empty the Trash. The original puts the row in the Trash view's own menu
    /// and greys it out while the Trash is empty (`FUN_0018db20` adds
    /// `"@action:inmenu" "Empty Trash"` at 0x8e6cf and then calls `setEnabled`
    /// with the negation of the emptiness test at `FUN_001a3750`). The question
    /// before it is the third of the original's Confirmations keys,
    /// `ConfirmEmptyTrash` (written by `FUN_001283f0` at 0x1285e9 into the
    /// `Confirmations` group of `kiorc`); it ships on here, because nothing can
    /// be brought back from this one.
    pub fn ask_or_empty_trash(&mut self) {
        let items = self.pane().entries.len();
        if items == 0 && self.pane().is_trash() {
            self.toast("The Trash is already empty.");
            return;
        }
        if self.config.confirm_empty_trash {
            self.confirm = Some(Confirm {
                title: "Empty Trash".to_string(),
                body: if items > 0 && self.pane().is_trash() {
                    format!(
                        "Empty the Trash and delete its {items} item{} for good?",
                        if items == 1 { "" } else { "s" }
                    )
                } else {
                    "Empty the Trash and delete everything in it for good?".to_string()
                },
                action: ConfirmAction::EmptyTrash,
            });
            return;
        }
        self.empty_trash_now();
    }

    /// The deed behind both routes into it. The Trash view is reloaded so the
    /// rows the person was looking at disappear at once.
    pub fn empty_trash_now(&mut self) {
        // No offer to undo survives clearing the Trash: whatever was put back
        // before this is gone with the rest.
        self.undo_trash = None;
        match fsutil::empty_trash() {
            Ok(()) => {
                self.toast("Trash emptied.");
                self.reload_after_delete();
            }
            Err(e) => self.toast(&format!("Could not empty the Trash: {e}")),
        }
    }

    /// Put back whatever the toast is still offering. Called by its Undo button.
    pub fn undo_last_trash(&mut self) {
        let Some(undo) = self.undo_trash.take() else {
            return;
        };
        let back = fsutil::restore_trashed(&undo.records);
        match back {
            0 => self.toast("Nothing came back — the items are no longer in the Trash."),
            1 => {
                self.toast("Put back. The item is where it was.");
                self.reload_after_delete();
            }
            n => {
                self.toast(&format!("Put back. {n} items are where they were."));
                self.reload_after_delete();
            }
        }
    }

    /// A deletion changes the folder, so every pane — both halves of a split and
    /// every tab — is read again, and nothing stays marked that is no longer
    /// there.
    pub fn reload_after_delete(&mut self) {
        for tab in self.tabs.iter_mut() {
            for pane in tab.panes.iter_mut() {
                pane.reload();
                pane.sel.clear();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shortcuts in the navigation panel
    // -----------------------------------------------------------------------

    /// Every row of the navigation panel: the standard folders first, then the
    /// shortcuts the person pinned. The order is the order of the Alt+1…9 keys.
    pub fn shortcut_entries(&self) -> Vec<(String, PathBuf, theme::PlaceIcon)> {
        let mut rows: Vec<(String, PathBuf, theme::PlaceIcon)> = vec![
            ("Home".into(), fsutil::home_dir(), theme::PlaceIcon::Home),
            (
                "Desktop".into(),
                fsutil::xdg_dir("DESKTOP", "Desktop"),
                theme::PlaceIcon::Desktop,
            ),
            (
                "Documents".into(),
                fsutil::xdg_dir("DOCUMENTS", "Documents"),
                theme::PlaceIcon::Documents,
            ),
            (
                "Downloads".into(),
                fsutil::xdg_dir("DOWNLOAD", "Downloads"),
                theme::PlaceIcon::Downloads,
            ),
            (
                "Music".into(),
                fsutil::xdg_dir("MUSIC", "Music"),
                theme::PlaceIcon::Music,
            ),
            (
                "Pictures".into(),
                fsutil::xdg_dir("PICTURES", "Pictures"),
                theme::PlaceIcon::Pictures,
            ),
            (
                "Videos".into(),
                fsutil::xdg_dir("VIDEOS", "Videos"),
                theme::PlaceIcon::Videos,
            ),
            (
                "Trash".into(),
                PathBuf::from(TRASH_URI),
                theme::PlaceIcon::Trash,
            ),
        ];
        for bm in &self.config.bookmarks {
            rows.push((
                bm.name.clone(),
                bm.path.clone(),
                theme::PlaceIcon::Bookmark,
            ));
        }
        rows
    }

    /// Number of standard rows before the person's own shortcuts start.
    pub fn standard_rows(&self) -> usize {
        self.shortcut_entries().len() - self.config.bookmarks.len()
    }

    // -----------------------------------------------------------------------
    // Added feature: the two recent groups in the navigation panel
    // -----------------------------------------------------------------------

    /// Read the desktop's recently used list again when the copy in hand has
    /// gone stale. The list only changes when another program opens a file, so
    /// a few seconds of age costs nothing and re-reading it every frame would
    /// touch the disk sixty times a second.
    pub fn refresh_recent_files(&mut self) {
        let stale = match self.recent_files_at {
            None => true,
            Some(at) => at.elapsed() > std::time::Duration::from_secs(3),
        };
        if !stale {
            return;
        }
        self.recent_files_at = Some(Instant::now());
        let limit = self.config.recent_files_shown.max(1);
        self.recent_files = fsutil::recent_files(limit);
    }

    /// Note the folder now in front, newest first, so *Recent Locations* holds
    /// the way back to it. The list belongs to the window rather than to the
    /// desktop's own recent list: walking into a folder is not "using a file",
    /// and the two groups should not say the same thing.
    pub fn remember_location(&mut self, path: &Path) {
        if !path.is_dir() {
            return;
        }
        let mut list = self.config.recent_locations.clone();
        note_location(&mut list, path, RECENT_LOCATIONS_MAX);
        if list != self.config.recent_locations {
            self.config.recent_locations = list;
            self.config_dirty = true;
        }
    }

    /// Give one shortcut row a colour, or put it back to normal with `None`.
    /// The choice is remembered in the config, next to the shortcuts.
    pub fn set_shortcut_color(&mut self, path: &std::path::Path, rgb: Option<[u8; 3]>) {
        if self.config.shortcut_color(path) == rgb {
            return;
        }
        self.config.set_shortcut_color(path, rgb);
        self.config_dirty = true;
        match rgb {
            Some(rgb) => {
                let name = foldercolor::name_of(rgb).unwrap_or("coloured");
                self.toast(&format!("Shortcut colour: {name}"));
            }
            None => self.toast("Shortcut back to its normal colour"),
        }
    }

    pub fn open_shortcut_editor(&mut self, original: Option<PathBuf>, path: PathBuf) {
        let is_folder = path.is_dir();
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        self.shortcut_edit = Some(ShortcutEdit {
            name,
            path: path.display().to_string(),
            focus: false,
            error: if is_folder {
                None
            } else {
                Some("That is not a folder.".to_string())
            },
            original,
        });
    }

    /// Save the shortcut editor: update the renamed one or add a new row.
    pub fn save_shortcut(&mut self) {
        let Some(edit) = self.shortcut_edit.take() else {
            return;
        };
        let name = edit.name.trim().to_string();
        let path = PathBuf::from(edit.path.trim());
        if name.is_empty() {
            self.shortcut_edit = Some(ShortcutEdit {
                error: Some("Give the shortcut a name.".to_string()),
                ..edit
            });
            return;
        }
        if !path.is_dir() {
            self.shortcut_edit = Some(ShortcutEdit {
                error: Some("That folder does not exist.".to_string()),
                ..edit
            });
            return;
        }
        // A folder that is already one of the standard rows does not need a
        // second row: point at the existing one instead of duplicating it.
        let already_listed = self
            .shortcut_entries()
            .iter()
            .take(self.standard_rows())
            .any(|(_, p, _)| *p == path);
        if already_listed && edit.original.is_none() {
            self.shortcut_edit = Some(ShortcutEdit {
                error: Some("That folder is already in the panel — click its row instead.".to_string()),
                ..edit
            });
            return;
        }
        match edit.original {
            Some(old) => {
                if let Some(bm) = self.config.bookmarks.iter_mut().find(|b| b.path == old) {
                    bm.name = name;
                    bm.path = path;
                }
                self.toast("Shortcut renamed");
            }
            None => {
                if self.config.bookmarks.iter().any(|b| b.path == path) {
                    self.config
                        .bookmarks
                        .iter_mut()
                        .find(|b| b.path == path)
                        .map(|b| b.name = name);
                    self.toast("Shortcut updated");
                } else {
                    self.config
                        .bookmarks
                        .push(config::Bookmark { name, path });
                    self.toast("Shortcut added");
                }
                self.config.shortcuts_expanded = true;
            }
        }
        self.config_dirty = true;
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // The picture viewer is a layer of its own: while it is open it owns the
        // keyboard, so no file-manager shortcut can fire behind it.
        if self.viewer.is_some() {
            self.viewer_keys(ctx);
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            if self.guide_open {
                self.guide_open = false;
            } else if self.quick.is_some() {
                self.quick = None;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.bulk.is_some() {
                self.bulk = None;
            } else if self.dups.is_some() {
                self.dups = None;
            } else if self.new_folder.is_some() {
                self.new_folder = None;
            } else if self.open_with.is_some() {
                self.open_with = None;
            } else if self.props.is_some() {
                self.props = None;
            } else if self.shortcut_edit.is_some() {
                self.shortcut_edit = None;
            }
        }
        if typing {
            return;
        }

        let mut reqs: Vec<Request> = Vec::new();
        ctx.input_mut(|i| {
            use egui::Key;
            let ctrl = Modifiers::CTRL;
            let ctrl_shift = Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            };
            let alt = Modifiers::ALT;
            if i.consume_key(ctrl, Key::T) {
                reqs.push(Request::Toast("__newtab__".into()));
            }
            if i.consume_key(ctrl, Key::W) {
                reqs.push(Request::Toast("__closetab__".into()));
            }
            if i.consume_key(ctrl, Key::N) {
                reqs.push(Request::Toast("__newwin__".into()));
            }
            if i.consume_key(ctrl, Key::Q) {
                reqs.push(Request::Toast("__quit__".into()));
            }
            if i.consume_key(ctrl_shift, Key::T) {
                // Reopen the tab that was closed last, which is what the
                // original's "Recently Closed Tabs" row does; Ctrl+T already
                // makes a new one.
                reqs.push(Request::Toast("__reopen__".into()));
            }
            if i.consume_key(ctrl, Key::L) {
                reqs.push(Request::Toast("__editloc__".into()));
            }
            if i.consume_key(ctrl, Key::H) {
                reqs.push(Request::Toast("__hidden__".into()));
            }
            if i.consume_key(ctrl, Key::F) {
                reqs.push(Request::Toast("__search__".into()));
            }
            // Ctrl+Z takes back the last Move to Trash, the way the original's
            // own Undo does ("This undoes the last change you made to files or
            // folders").
            if i.consume_key(ctrl, Key::Z) {
                reqs.push(Request::Toast("__undo__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F1) {
                reqs.push(Request::Toast("__guide__".into()));
            }
            if i.consume_key(ctrl, Key::I) {
                reqs.push(Request::Toast("__filter__".into()));
            }
            if i.consume_key(ctrl_shift, Key::D) {
                reqs.push(Request::Toast("__dups__".into()));
            }
            if i.consume_key(ctrl_shift, Key::R) {
                reqs.push(Request::Toast("__bulk__".into()));
            }
            if i.consume_key(ctrl_shift, Key::S) {
                reqs.push(Request::Toast("__addshortcut__".into()));
            }
            // Alt+1 … Alt+9 jump to the shortcuts in the navigation panel.
            for (n, key) in [
                (1u8, Key::Num1),
                (2, Key::Num2),
                (3, Key::Num3),
                (4, Key::Num4),
                (5, Key::Num5),
                (6, Key::Num6),
                (7, Key::Num7),
                (8, Key::Num8),
                (9, Key::Num9),
            ] {
                if i.consume_key(alt, key) {
                    reqs.push(Request::Toast(format!("__sc_{n}__")));
                }
            }
            if i.consume_key(ctrl, Key::C) {
                reqs.push(Request::Copy);
            }
            if i.consume_key(ctrl, Key::X) {
                reqs.push(Request::Cut);
            }
            if i.consume_key(ctrl, Key::V) {
                reqs.push(Request::Paste);
            }
            if i.consume_key(ctrl, Key::A) {
                reqs.push(Request::Toast("__selectall__".into()));
            }
            if i.consume_key(ctrl, Key::Plus) || i.consume_key(ctrl, Key::Equals) {
                reqs.push(Request::Toast("__zoomin__".into()));
            }
            if i.consume_key(ctrl, Key::Minus) {
                reqs.push(Request::Toast("__zoomout__".into()));
            }
            if i.consume_key(ctrl, Key::Num0) {
                reqs.push(Request::Toast("__zoomreset__".into()));
            }
            if i.consume_key(alt, Key::ArrowLeft) {
                reqs.push(Request::Toast("__back__".into()));
            }
            if i.consume_key(alt, Key::ArrowRight) {
                reqs.push(Request::Toast("__forward__".into()));
            }
            if i.consume_key(alt, Key::ArrowUp) {
                reqs.push(Request::Toast("__up__".into()));
            }
            if i.consume_key(alt, Key::Enter) {
                reqs.push(Request::Toast("__props__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F2) {
                reqs.push(Request::Toast("__rename__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F3) {
                reqs.push(Request::Toast("__split__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F4) {
                reqs.push(Request::Toast("__pictures__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                reqs.push(Request::Toast("__refresh__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F9) {
                reqs.push(Request::Toast("__places__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F10) {
                reqs.push(Request::Toast("__newfolder__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::F11) {
                reqs.push(Request::Toast("__info__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::Space) {
                reqs.push(Request::Toast("__quick__".into()));
            }
            if i.consume_key(Modifiers::SHIFT, Key::Delete) {
                reqs.push(Request::Toast("__delete__".into()));
            } else if i.consume_key(Modifiers::NONE, Key::Delete) {
                reqs.push(Request::Toast("__trash__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                reqs.push(Request::Toast("__open__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                reqs.push(Request::Toast("__down__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                reqs.push(Request::Toast("__upkey__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::Home) {
                reqs.push(Request::Toast("__homekey__".into()));
            }
            if i.consume_key(Modifiers::NONE, Key::End) {
                reqs.push(Request::Toast("__endkey__".into()));
            }
        });

        for req in reqs {
            match req {
                Request::Toast(tag) if tag == "__quit__" => {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    break;
                }
                Request::Toast(tag) => self.apply_tag(&tag),
                Request::Copy => {
                    let paths = self.pane().selected_paths();
                    if !paths.is_empty() {
                        self.clipboard.paths = paths;
                        self.clipboard.cut = false;
                        self.toast("Copied to clipboard");
                    }
                }
                Request::Cut => {
                    let paths = self.pane().selected_paths();
                    if !paths.is_empty() {
                        self.clipboard.paths = paths;
                        self.clipboard.cut = true;
                        self.toast("Cut to clipboard");
                    }
                }
                other => self.reqs.push(other),
            }
        }
    }

    /// A short description of the open tabs: which folder each tab is showing
    /// and which tab is in front. The settings file is only rewritten when this
    /// changes or a setting changed, so a still window does not touch the disk.
    fn session_signature(&self) -> String {
        let mut sig = String::new();
        for tab in &self.tabs {
            sig.push_str(&tab.panes[tab.active].current_dir().to_string_lossy());
            sig.push('\u{1f}');
        }
        sig.push_str(&self.active_tab.to_string());
        sig
    }

    /// Copy the live window state into the settings file. Called when a setting
    /// changed, when the session changed, and once when the window closes, so
    /// the next start opens the way this one was left: same sort, same view and
    /// zoom, same tabs, same window size and place.
    fn persist(&mut self, ctx: Option<&egui::Context>) {
        if self.shot.is_some() {
            return;
        }
        let cfg = self.config_from_state(ctx);
        cfg.save();
        self.settings_sig = settings_signature(&cfg);
        self.config = cfg;
        self.session_sig = self.session_signature();
        self.config_dirty = false;
    }

    /// Everything the next start should open with, taken from the live window.
    /// Kept apart from `persist` so the same snapshot can be compared with the
    /// last one written: if anything in it changed, it is worth saving, whether
    /// or not the part that changed remembered to ask for a save.
    fn config_from_state(&self, ctx: Option<&egui::Context>) -> Config {
        let mut cfg = self.config.clone();
        // Split is a choice for the window you are in, not a setting: it is
        // neither restored nor written back, so the file cannot claim a start
        // will be two views.
        cfg.split = false;
        cfg.places_visible = self.places_visible;
        cfg.info_panel = self.info_panel;
        cfg.show_hidden = self.pane().show_hidden;
        cfg.previews = self.pane().previews;
        cfg.view = self.pane().view.index();
        cfg.zoom = self.pane().zoom;
        cfg.zoom_plain = self.pane().zoom_plain;
        cfg.folders_first = self.pane().folders_first;
        cfg.name_sort = self.pane().name_sort.index();
        cfg.sort_key = self.pane().sort.index();
        cfg.sort_desc = !self.pane().asc;
        cfg.split_frac = self.split_frac;
        cfg.pictures = self.pane().pictures;
        cfg.thumb_px = self.pane().thumb_px;
        cfg.gallery_names = self.pane().gallery_names;
        cfg.gallery_dims = self.pane().gallery_dims;
        cfg.gallery_round = self.pane().gallery_round;
        cfg.last_dir = self.pane().current_dir().display().to_string();
        // The session, in tab order. The active pane of each tab is the folder
        // that tab stands for.
        cfg.session = self
            .tabs
            .iter()
            .map(|t| {
                let pane = &t.panes[t.active];
                SavedTab {
                    path: pane.current_dir(),
                    sort_key: Some(pane.sort.index()),
                    sort_desc: Some(!pane.asc),
                }
            })
            .collect();
        cfg.session_tab = self.active_tab;
        if let Some(ctx) = ctx {
            let (size, pos) = ctx.input(|i| {
                let vp = i.viewport();
                (vp.inner_rect.map(|r| [r.width(), r.height()]), vp.outer_rect.map(|r| r.min))
            });
            if let Some(size) = size {
                cfg.window_px = Some([size[0].round(), size[1].round()]);
            }
            if let Some(pos) = pos {
                cfg.window_pos = Some([pos.x.round(), pos.y.round()]);
            }
        }
        cfg
    }

    fn apply_tag(&mut self, tag: &str) {
        match tag {
            "__newtab__" => {
                let dir = self.pane().current_dir();
                self.new_tab_public(dir);
            }
            "__newwin__" => {
                let dir = self.pane().current_dir();
                spawn_folder_window(&dir);
            }
            "__closetab__" => {
                let idx = self.active_tab;
                self.close_tab(idx);
            }
            // Ctrl+Shift+T: the tab that was closed last comes back. The
            // original keeps the list under the menu's "Recently Closed Tabs"
            // (0x12d8ff in /usr/bin/dolphin).
            "__reopen__" => {
                self.reqs.push(Request::ReopenClosedTab(0));
            }
            "__editloc__" => {
                self.pane_mut().begin_location_edit();
            }
            "__hidden__" => {
                let show = !self.pane().show_hidden;
                self.pane_mut().show_hidden = show;
                self.config.show_hidden = show;
                self.config_dirty = true;
                self.pane_mut().reload();
            }
            "__search__" => {
                let pane = self.pane_mut();
                pane.filter_on = true;
                pane.find_deep = false;
                self.filter_focus = true;
            }
            "__guide__" => {
                self.guide_open = true;
            }
            "__filter__" => {
                let on = !self.pane().filter_on;
                let pane = self.pane_mut();
                pane.filter_on = on;
                if !on {
                    pane.filter.clear();
                    pane.search = None;
                    pane.refilter();
                }
                self.filter_focus = on;
            }
            "__selectall__" => self.pane_mut().select_all(),
            "__zoomin__" => {
                self.pane_mut().zoom_by(1);
                self.config.zoom = self.pane().zoom;
                self.config.zoom_plain = self.pane().zoom_plain;
            }
            "__zoomout__" => {
                self.pane_mut().zoom_by(-1);
                self.config.zoom = self.pane().zoom;
                self.config.zoom_plain = self.pane().zoom_plain;
            }
            "__zoomreset__" => {
                self.pane_mut().zoom = [4, 0, 0];
                self.pane_mut().zoom_plain = [2, 0, 0];
                self.config.zoom = self.pane().zoom;
                self.config.zoom_plain = self.pane().zoom_plain;
            }
            "__back__" => self.pane_mut().go_back(),
            "__forward__" => self.pane_mut().go_forward(),
            "__up__" => self.pane_mut().go_up(),
            "__refresh__" => self.pane_mut().reload(),
            "__pictures__" => {
                // Added feature: the picture gallery on and off (F4).
                let pane = self.pane_mut();
                let on = !pane.pictures;
                pane.set_pictures(on);
                self.config.pictures = pane.pictures;
                self.config_dirty = true;
            }
            "__split__" => {
                if self.split {
                    self.unsplit();
                } else {
                    self.ensure_split();
                }
                self.config_dirty = true;
            }
            "__places__" => self.places_visible = !self.places_visible,
            "__info__" => self.info_panel = !self.info_panel,
            "__newfolder__" => {
                self.new_folder = Some(NewFolder {
                    name: "New Folder".to_string(),
                    focus: false,
                });
            }
            "__rename__" => {
                let idx = self.pane().cursor;
                if let Some(entry) = self.pane().display_entries().get(idx).cloned() {
                    let base = entry
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| entry.name.clone());
                    self.pane_mut().rename = Some(state::RenameState {
                        index: idx,
                        text: base,
                        focus: false,
                    });
                }
            }
            "__quick__" => {
                if let Some(e) = self.pane().primary_selection().cloned() {
                    self.reqs.push(Request::QuickLook(e.path));
                }
            }
            "__undo__" => {
                if self.undo_trash.is_some() {
                    self.undo_last_trash();
                } else {
                    self.toast("Nothing to undo.");
                }
            }
            "__trash__" => {
                let paths = self.pane().selected_paths();
                if paths.is_empty() {
                    self.toast("Nothing is selected.");
                } else {
                    self.reqs.push(Request::Trash(paths));
                }
            }
            "__delete__" => {
                let paths = self.pane().selected_paths();
                if paths.is_empty() {
                    self.toast("Nothing is selected.");
                } else {
                    self.reqs.push(Request::DeletePermanent(paths));
                }
            }
            "__props__" => {
                let paths = self.pane().selected_paths();
                if !paths.is_empty() {
                    self.reqs.push(Request::Properties(paths));
                }
            }
            "__open__" => {
                let idx = self.pane().cursor;
                let mut reqs = Vec::new();
                self.pane_mut().open_entry(idx, &mut reqs);
                self.reqs.extend(reqs);
            }
            "__down__" => self.move_cursor(1),
            "__upkey__" => self.move_cursor(-1),
            "__homekey__" => {
                self.pane_mut().cursor = 0;
                self.pane_mut().select_single(0);
                self.pane_mut().scroll_to = Some(0);
            }
            "__endkey__" => {
                let last = self.pane().display_entries().len().saturating_sub(1);
                self.pane_mut().cursor = last;
                self.pane_mut().select_single(last);
                self.pane_mut().scroll_to = Some(last);
            }
            "__dups__" => {
                let root = self.pane().current_dir();
                self.dups = Some(Dups {
                    scan: DupScan::start(root),
                });
            }
            "__addshortcut__" => {
                let dir = self.pane().current_dir();
                self.open_shortcut_editor(None, dir);
            }
            "__bulk__" => {
                let paths = self.pane().selected_paths();
                if paths.is_empty() {
                    self.toast("Select the files to rename first.");
                } else {
                    self.bulk = Some(Bulk {
                        paths,
                        pattern: "{name}{ext}".to_string(),
                        start: 1,
                        error: None,
                    });
                }
            }
            tag if tag.starts_with("__sc_") => {
                let n: usize = tag
                    .trim_start_matches("__sc_")
                    .trim_end_matches("__")
                    .parse()
                    .unwrap_or(0);
                if n >= 1 {
                    let rows = self.shortcut_entries();
                    if let Some((_, path, _)) = rows.get(n - 1) {
                        let path = path.clone();
                        self.pane_mut().navigate(path);
                    }
                }
            }
            _ => {}
        }
        self.config_dirty = true;
    }

    fn move_cursor(&mut self, delta: i32) {
        // The arrow keys move through what the view actually shows: on the
        // picture wall that is the pictures, never the documents between them.
        let shown = self.pane().shown_indices();
        if shown.is_empty() {
            return;
        }
        let at = self.pane().cursor;
        let pos = shown.iter().position(|i| *i == at);
        let next = match pos {
            Some(p) => shown[(p as i32 + delta).clamp(0, shown.len() as i32 - 1) as usize],
            None if delta > 0 => shown[0],
            None => *shown.last().unwrap(),
        };
        self.pane_mut().cursor = next;
        self.pane_mut().select_single(next);
        self.pane_mut().scroll_to = Some(next);
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() {
            return;
        }
        self.frames += 1;
        ctx.request_repaint();
        let mut image = None;
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Screenshot { image: img, .. } = event {
                    image = Some(img.clone());
                }
            }
        });
        let path = self.shot.as_ref().unwrap().path.clone();
        if let Some(img) = image {
            let [w, h] = img.size;
            let mut rgba = Vec::with_capacity(w * h * 4);
            for px in &img.pixels {
                rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let ok = match image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8) {
                Ok(()) => {
                    eprintln!("screenshot saved to {}", path.display());
                    true
                }
                Err(e) => {
                    eprintln!("screenshot failed: {}", e);
                    false
                }
            };
            // Leave at once. Asking the viewport to close usually ends the run,
            // but the window manager may never answer when the window was never
            // given focus; a captured picture must always end the process so the
            // caller is not left waiting on it.
            let _ = ctx;
            std::process::exit(if ok { 0 } else { 1 });
        }
        let requested = self.shot.as_ref().unwrap().requested;
        if !requested && self.frames >= 120 {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            if let Some(shot) = self.shot.as_mut() {
                shot.requested = true;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // A picture named on the command line opens on the very first frame, so
        // a captured window shows the viewer and not an empty desktop.
        if let Some((path, play)) = self.pending_viewer.take() {
            if path.is_file() {
                self.open_viewer(&path, play);
                if self.open_details_at_start {
                    if let Some(v) = self.viewer.as_mut() {
                        v.show_info = true;
                    }
                }
            }
        }
        // A comparison asked for on the command line is asked for before the
        // folder has been read, so the viewer may still hold only the picture it
        // was opened with. Keep the request until the list has a second picture
        // instead of dropping it on the first frame; then it is applied once.
        if let Some(mode) = self.pending_compare {
            let mut applied = false;
            if let Some(viewer) = self.viewer.as_mut() {
                if viewer.items.len() >= 2 && viewer.start_compare() {
                    if let Some(cmp) = viewer.compare.as_mut() {
                        cmp.mode = mode;
                    }
                    applied = true;
                }
            }
            if applied || self.viewer.is_none() || self.frames > 240 {
                self.pending_compare = None;
            }
        }
        self.handle_shortcuts(ctx);
        // Added feature: every folder that comes to the front is noted for the
        // navigation panel's Recent Locations group. The Trash is not a folder
        // with a way back — the pane reports the home folder for it — so it is
        // left out rather than filling the group with Home.
        if !self.pane().is_trash() {
            let dir = self.pane().current_dir();
            self.remember_location(&dir);
        }
        // `--open-with <file>`: open the chooser once the window is up, so the
        // dialog can be photographed.
        if let Some(file) = self.pending_open_with.clone() {
            if self.frames > 2 {
                self.pending_open_with = None;
                self.reqs.push(Request::OpenWithChoose(vec![file]));
            }
        }
        // `--trash-demo <folder>`: mark everything in that folder and take it to
        // the Trash once the listing is there, so the toast and its Undo button
        // can be photographed. It never deletes anything for good.
        if let Some(dir) = self.pending_trash_demo.clone() {
            let here = self.pane().current_dir();
            if here == dir && !self.pane().entries.is_empty() {
                let paths: Vec<PathBuf> = self.pane().entries.iter().map(|e| e.path.clone()).collect();
                self.pane_mut().sel = (0..paths.len()).collect();
                self.trash_now(paths);
                self.pending_trash_demo = None;
            } else if self.frames > 180 {
                self.pending_trash_demo = None;
            }
        }
        self.top_toolbar(ctx);
        self.tab_strip(ctx);
        self.places_panel(ctx);
        self.info_panel_ui(ctx);
        self.status_bar(ctx);
        self.central(ctx);
        self.handle_requests(ctx);
        self.toast_ui(ctx);
        self.dialogs(ctx);
        // The viewer is drawn last, over everything, the way the browser's
        // viewer covers its page.
        self.viewer_ui(ctx);
        self.thumbs.poll(ctx);
        self.big.poll(ctx);

        // A captured picture must not rewrite the person's settings. Outside a
        // capture, the settings file is written whenever a setting changed, and
        // about twice a second it is compared with the live window as a whole:
        // anything that differs is written, so a choice made by a button that
        // forgot to announce itself still survives a crash as well as a tidy
        // exit.
        if self.shot.is_none() {
            if self.frames % 30 == 0 {
                let sig = settings_signature(&self.config_from_state(Some(ctx)));
                if sig != self.settings_sig {
                    self.config_dirty = true;
                }
            }
            if self.config_dirty {
                self.persist(Some(ctx));
            }
        }
        self.screenshot_tick(ctx);
    }

    /// The window is closing: one last write, so the tabs, the sort and the
    /// window size are current even if nothing was marked dirty since the last
    /// save. The original does the same in `DolphinMainWindow::slotSaveSession`.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist(None);
    }
}

// ---------------------------------------------------------------------------
// Small UI helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Status bar wording
// ---------------------------------------------------------------------------

/// The words the original puts in the status bar. `DolphinView::emitStatusBarText`
/// @0x273180 in libdolphinprivate builds three shapes from the message ids that
/// sit in the same library:
///
/// * `0 folders, 0 files` (`0x2bf6ea`) when the folder holds nothing;
/// * `%1 folders` / `%1 files` (`0x2bf6a0`, `0x2bf6b4`, singular written as
///   `1 folder` / `1 file` by the plural form);
/// * `%1, %2 (%3)` (`0x2bf6c4`) for folders + files + size, or `%1 (%2)`
///   (`0x2becea`) when there are only files.
///
/// The size is `KFormat::formatByteSize(value, 1)` — one decimal, as in the
/// capture of the installed program ("15 files (5.7 KiB)").
pub(crate) fn status_counts(folders: usize, files: usize, bytes: u64) -> String {
    if folders == 0 && files == 0 {
        return "0 folders, 0 files".to_string();
    }
    let folder_word = if folders == 1 {
        "1 folder".to_string()
    } else {
        format!("{folders} folders")
    };
    if files == 0 {
        return folder_word;
    }
    let file_word = if files == 1 {
        "1 file".to_string()
    } else {
        format!("{files} files")
    };
    if folders == 0 {
        return format!("{file_word} ({})", fsutil::format_size(bytes));
    }
    format!("{folder_word}, {file_word} ({})", fsutil::format_size(bytes))
}

/// The selection wording from the same function: `%1 folders selected`
/// (`0x1bf658`), `%1 files selected` (`0x1bf67e`), `1 folder selected`
/// (`0x1bf66c`) and `1 file selected` (`0x1bf690`).
///
/// The function runs the *same* composition as the unselected line: folders
/// only → the folder phrase alone (no size), files only → `%1 (%2)` (1becea),
/// both → `%1, %2 (%3)` (0x1bf6c4). So a selection of files always carries the
/// size of the selected files, and a mixed selection reads
/// "1 folder selected, 2 files selected (12.0 KiB)" rather than being collapsed
/// to "%1 items selected" — that string belongs to the Information panel
/// (`FUN_001f76f0` @0x1f76f0), not to the status bar.
pub(crate) fn status_selection(folders: usize, files: usize, bytes: u64) -> String {
    let folder_word = if folders == 1 {
        "1 folder selected".to_string()
    } else {
        format!("{folders} folders selected")
    };
    if files == 0 {
        if folders == 0 {
            return "0 folders, 0 files".to_string();
        }
        return folder_word;
    }
    let file_word = if files == 1 {
        "1 file selected".to_string()
    } else {
        format!("{files} files selected")
    };
    if folders == 0 {
        return format!("{file_word} ({})", fsutil::format_size(bytes));
    }
    format!("{folder_word}, {file_word} ({})", fsutil::format_size(bytes))
}

/// The pressed/hover plate under a toolbar control, shared by all three kinds
/// so an icon button, a button with a word and a split button look the same.
fn toolbar_plate(ui: &egui::Ui, rect: Rect, resp: &egui::Response, active: bool, enabled: bool) {
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(63, 70, 77));
    } else if resp.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), ROW_HOVER);
    } else if resp.is_pointer_button_down_on() && enabled {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(38, 43, 47));
    }
}

/// A toolbar button that can spell out its own verb: the icon stays in a
/// 30 px column and the word follows it. `label` is `None` in a narrow window,
/// where the row falls back to the original's icon-only look.
fn toolbar_word_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: Option<&str>,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let text_w = label
        .map(|t| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(t.to_string(), FontId::proportional(12.0), TEXT)
                    .size()
                    .x
            })
        })
        .unwrap_or(0.0);
    let width = if label.is_some() {
        TOOLBAR_ICON_W + text_w + TOOLBAR_LABEL_PAD
    } else {
        TOOLBAR_ICON_W
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, TOOLBAR_ICON_W), Sense::click());
    toolbar_plate(ui, rect, &resp, active, enabled);
    let tint = if !enabled {
        TEXT_FAINT
    } else if active {
        ACCENT
    } else {
        TEXT
    };
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + TOOLBAR_ICON_W * 0.5, rect.center().y),
        Vec2::splat(16.0),
    );
    theme::draw_glyph(ui.painter(), icon_rect, glyph, tint);
    if let Some(text) = label {
        ui.painter().text(
            Pos2::new(rect.left() + TOOLBAR_ICON_W, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            tint,
        );
    }
    resp
}

fn toolbar_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::click());
    let hovered = resp.hovered() && enabled;
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), ROW_HOVER);
    }
    if resp.is_pointer_button_down_on() && enabled {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(38, 43, 47));
    }
    theme::draw_glyph(
        ui.painter(),
        rect,
        glyph,
        if enabled { TEXT } else { TEXT_FAINT },
    );
    resp
}

fn toolbar_button_active(ui: &mut egui::Ui, glyph: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::click());
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(63, 70, 77));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), ROW_HOVER);
    }
    theme::draw_glyph(
        ui.painter(),
        rect,
        glyph,
        if active { ACCENT } else { TEXT },
    );
    resp
}

fn small_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(13.0))
            .fill(Color32::TRANSPARENT)
            .min_size(Vec2::new(18.0, 18.0)),
    )
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text)
            .size(10.5)
            .color(TEXT_DIM),
    );
    ui.add_space(2.0);
}

/// A group heading in the navigation panel that folds its rows away, drawn in
/// the same shape as the Shortcuts heading: a chevron, the name, and how many
/// rows are inside. Returns true when the heading itself was clicked.
fn recent_group_header(ui: &mut egui::Ui, text: &str, expanded: bool, count: usize) -> bool {
    ui.add_space(4.0);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(1.0, 1.0)), CornerRadius::same(3), ROW_HOVER);
    }
    let chev = Rect::from_center_size(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        Vec2::splat(12.0),
    );
    theme::draw_glyph(
        ui.painter(),
        chev,
        if expanded { "chevron_down" } else { "chevron_right" },
        TEXT_DIM,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 19.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.5),
        TEXT_DIM,
    );
    if count > 0 {
        ui.painter().text(
            Pos2::new(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{count}"),
            FontId::proportional(10.0),
            TEXT_FAINT,
        );
    }
    resp.clicked()
}

/// A muted line where a group has no rows: the original's own emptiness
/// wording, so an empty group never looks like a broken one.
fn quiet_row(ui: &mut egui::Ui, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.0),
        TEXT_FAINT,
    );
}

fn place_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: theme::PlaceIcon,
    selected: bool,
) -> egui::Response {
    row_impl(ui, label, Some(icon), selected, None)
}

fn tool_row(ui: &mut egui::Ui, label: &str, tooltip: &str, key: Option<&str>) -> bool {
    let resp = row_impl(ui, label, Some(theme::PlaceIcon::Tools), false, key);
    resp.on_hover_text(tooltip).clicked()
}

/// One row of the Shortcuts section, with the key that jumps to it drawn at
/// the right edge of the row. `colour` paints the row's little icon — the flag
/// for a pinned shortcut — in the colour the person chose for it. The colour
/// stays when the row is marked (selected); only its shade is lifted so it
/// reads on the selection band.
fn shortcut_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: theme::PlaceIcon,
    selected: bool,
    key: Option<&str>,
    colour: Option<[u8; 3]>,
) -> egui::Response {
    let tint = colour.map(|c| Color32::from_rgb(c[0], c[1], c[2]));
    row_tinted(ui, label, Some(icon), selected, key, tint)
}

/// A "Shortcut Colour" entry inside a context menu: the eight swatches the
/// folder menu offers, plus a way back to the normal colour.
fn colour_menu_item(
    ui: &mut egui::Ui,
    label: &str,
    current: Option<[u8; 3]>,
    pick: &mut Option<(PathBuf, Option<[u8; 3]>)>,
    path: &std::path::Path,
) {
    ui.menu_button(label, |ui| {
        ui.set_min_width(190.0);
        match theme::colour_menu(ui, current) {
            Some(theme::ColourPick::Set(rgb)) => {
                *pick = Some((path.to_path_buf(), Some(rgb)));
                ui.close();
            }
            Some(theme::ColourPick::Clear) => {
                *pick = Some((path.to_path_buf(), None));
                ui.close();
            }
            None => {}
        }
    });
}

/// Heading of the Shortcuts section: a fold arrow, the name, how many
/// shortcuts the person pinned and a "+" button that opens the editor.
fn shortcuts_header(ui: &mut egui::Ui, expanded: bool, pinned: usize) -> (bool, bool) {
    ui.add_space(4.0);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(1.0, 1.0)), CornerRadius::same(3), ROW_HOVER);
    }
    let chev = Rect::from_center_size(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        Vec2::splat(12.0),
    );
    theme::draw_glyph(
        ui.painter(),
        chev,
        if expanded { "chevron_down" } else { "chevron_right" },
        TEXT_DIM,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 19.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Shortcuts",
        FontId::proportional(10.5),
        TEXT_DIM,
    );
    if pinned > 0 {
        ui.painter().text(
            Pos2::new(rect.right() - 28.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{pinned} pinned"),
            FontId::proportional(10.0),
            TEXT_FAINT,
        );
    }
    let plus_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 11.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let plus = ui.interact(plus_rect, ui.id().with("shortcut_add"), Sense::click());
    if plus.hovered() {
        ui.painter()
            .rect_filled(plus_rect, CornerRadius::same(3), ROW_HOVER);
    }
    theme::draw_glyph(
        ui.painter(),
        plus_rect,
        "plus",
        if plus.hovered() { TEXT } else { TEXT_DIM },
    );
    let add = plus.on_hover_text("Add Shortcut (Ctrl+Shift+S)").clicked();
    (resp.on_hover_text("Fold the shortcuts away").clicked(), add)
}

/// The name a delete question shows for one item. The original puts the item's
/// name in the message body as `<filename>%1</filename>`
/// (`KIO::JobUiDelegate::askDeleteConfirmation` @0x1a8a20, kio 6.31.0).
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The list under a question about several items. The original passes the whole
/// selection to `KMessageBox::warningContinueCancelList`, which lists every name;
/// this keeps the first eight and counts the rest so the box stays a box.
fn name_list(paths: &[PathBuf]) -> String {
    const SHOWN: usize = 8;
    let mut lines: Vec<String> = paths.iter().take(SHOWN).map(|p| display_name(p)).collect();
    if paths.len() > SHOWN {
        lines.push(format!("and {} more", paths.len() - SHOWN));
    }
    format!("{} items:\n{}", paths.len(), lines.join("\n"))
}

/// The "Add Shortcut" row at the bottom of the Shortcuts section.
fn add_row(ui: &mut egui::Ui, label: &str) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(1.0, 1.0)), CornerRadius::same(3), ROW_HOVER);
    }
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 17.0, rect.center().y),
        Vec2::splat(15.0),
    );
    theme::draw_glyph(
        ui.painter(),
        icon_rect,
        "plus",
        if resp.hovered() { TEXT } else { TEXT_DIM },
    );
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        if resp.hovered() { TEXT } else { TEXT_DIM },
    );
    resp.on_hover_text("Pins the folder you are looking at (Ctrl+Shift+S)")
        .clicked()
}

/// The two rows whose wording must not promise a question this build no longer
/// asks. The original spells the first without an ellipsis too: its own binary
/// carries `Move to Trash` (0x12f039) and no `Delete Permanently` at all — that
/// one comes from KIO, whose action name has no ellipsis either. The person's
/// rule is that the mark and the key press are the whole action, so a trailing
/// "…" here would be a promise the row does not keep.
pub const DELETE_PERMANENT_LABEL: &str = "Delete Permanently";
pub const EMPTY_TRASH_LABEL: &str = "Empty Trash";

/// A command row inside a menu: the name on the left, its keyboard shortcut
/// on the right, the way the original shows them.
pub fn menu_row(ui: &mut egui::Ui, label: &str, hint: &str) -> egui::Response {
    let width = ui.available_width().max(140.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            CornerRadius::same(3),
            ROW_SEL_ACTIVE.gamma_multiply(0.9),
        );
    }
    ui.painter().text(
        Pos2::new(rect.left() + 9.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        TEXT,
    );
    if !hint.is_empty() {
        ui.painter().text(
            Pos2::new(rect.right() - 9.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            FontId::proportional(10.5),
            TEXT_FAINT,
        );
    }
    resp
}

/// The colour a place or shortcut glyph is drawn in.
///
/// A marked row keeps the colour the person gave it: the selection band gets
/// the same hue lifted towards white (see `theme::SELECTION_LIFT`). Rows with
/// no colour keep the usual blue-grey place colour, also lifted, instead of
/// turning white — so nothing loses its colour when it is marked.
fn place_icon_colour(selected: bool, tint: Option<Color32>) -> Color32 {
    let base = tint.unwrap_or(PLACE);
    if selected {
        theme::selection_tint(base)
    } else {
        base
    }
}

fn row_impl(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<theme::PlaceIcon>,
    selected: bool,
    key: Option<&str>,
) -> egui::Response {
    row_tinted(ui, label, icon, selected, key, None)
}

fn row_tinted(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<theme::PlaceIcon>,
    selected: bool,
    key: Option<&str>,
    tint: Option<Color32>,
) -> egui::Response {
    let width = ui.available_width();
    // Drag senses as well as clicks: the shortcut rows are dragged into a new
    // order, and egui only reports a drag for a row that asked for one. A press
    // and release without movement is still a click.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click_and_drag());
    if selected {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(1.0, 1.0)),
            CornerRadius::same(3),
            ROW_SEL_ACTIVE.gamma_multiply(0.75),
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(1.0, 1.0)),
            CornerRadius::same(3),
            ROW_HOVER,
        );
    }
    let mut text_x = rect.left() + 8.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 17.0, rect.center().y),
            Vec2::splat(17.0),
        );
        theme::draw_place_icon(
            ui.painter(),
            icon_rect,
            icon,
            place_icon_colour(selected, tint),
        );
        text_x = rect.left() + 32.0;
    }
    // The shortcut badge takes its own little space at the right edge, so the
    // name is shortened around it rather than underneath it.
    let key_w = key.map(|k| k.chars().count() as f32 * 5.0 + 12.0).unwrap_or(0.0);
    let font = FontId::proportional(12.5);
    let text = theme::elide(ui, label, rect.right() - text_x - 9.0 - key_w, font.clone());
    ui.painter().text(
        Pos2::new(text_x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        if selected { Color32::WHITE } else { TEXT },
    );
    if let Some(k) = key {
        let badge = Rect::from_min_max(
            Pos2::new(rect.right() - key_w - 6.0, rect.center().y - 8.5),
            Pos2::new(rect.right() - 6.0, rect.center().y + 8.5),
        );
        ui.painter().rect_filled(
            badge,
            CornerRadius::same(3),
            if selected {
                Color32::from_rgb(48, 57, 66)
            } else {
                Color32::from_rgb(36, 42, 48)
            },
        );
        ui.painter().text(
            badge.center(),
            Align2::CENTER_CENTER,
            k,
            FontId::monospace(9.0),
            if selected { TEXT } else { TEXT_DIM },
        );
    }
    resp
}

pub fn spawn_folder_window(path: &std::path::Path) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for item in rd.flatten() {
        let from = item.path();
        let to = dst.join(item.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn sort_search_results(entries: &mut [fsutil::Entry], key: state::SortKey, asc: bool) {
    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let ord = match key {
            state::SortKey::Name => fsutil::name_cmp(&a.name, &b.name),
            state::SortKey::Size => a.size.cmp(&b.size),
            state::SortKey::Type => a.type_label().cmp(&b.type_label()),
            state::SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if asc {
            ord
        } else {
            ord.reverse()
        }
    });
}

fn main() -> eframe::Result<()> {
    let mut screenshot: Option<PathBuf> = None;
    let mut start_dir: Option<PathBuf> = None;
    let mut start_zoom: Option<u8> = None;
    let mut start_view: Option<usize> = None;
    // `--keep-settings` takes the picture with the saved settings instead of
    // factory ones: the person's own shortcuts, colours and layout.
    let mut keep_settings = false;
    // Added feature: start in the picture gallery, or straight in the viewer on
    // a named picture (with `--play` running its slideshow).
    let mut start_pictures = false;
    let mut start_find: Option<String> = None;
    let mut start_deep = false;
    // `--below`: the picture wall looking inside the folders below.
    let mut start_below = false;
    let mut start_guide = false;
    let mut start_menu = false;
    let mut open_with_file: Option<PathBuf> = None;
    // Deep-pass captures: open the Sort By popup on the first frame, or open the
    // window already sorted by one key so the direction each key starts in can be
    // seen (see MECHANICS.md, "The Sort By menu, read out of the code").
    let mut start_sort_menu = false;
    let mut start_sort: Option<state::SortKey> = None;
    let mut trash_demo: Option<PathBuf> = None;
    // Window size, so a narrow layout can be looked at (and checked) without a
    // mouse: `--size 980x700`.
    let mut window_size: Option<[f32; 2]> = None;
    let mut start_viewer: Option<(PathBuf, bool)> = None;
    let mut play = false;
    let mut start_compare: Option<gallery::CompareMode> = None;
    let mut start_thumbs: Option<f32> = None;
    let mut start_dims = false;
    // Opens the viewer's details panel on the first picture; the panel is shut
    // unless this asks for it.
    let mut start_details = false;
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest::run());
    }
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" | "-s" => {
                if i + 1 < args.len() {
                    screenshot = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--dir" | "-d" => {
                if i + 1 < args.len() {
                    start_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            // Handy for pictures and for anyone starting the window at a
            // particular size: --zoom 8, --view icons|compact|details.
            "--zoom" | "-z" => {
                if i + 1 < args.len() {
                    start_zoom = args[i + 1].parse::<u8>().ok();
                    i += 1;
                }
            }
            "--view" | "-v" => {
                if i + 1 < args.len() {
                    start_view = match args[i + 1].to_ascii_lowercase().as_str() {
                        "icons" | "0" => Some(0),
                        "compact" | "1" => Some(1),
                        _ => Some(2),
                    };
                    i += 1;
                }
            }
            "--size" => {
                if i + 1 < args.len() {
                    if let Some((w, h)) = args[i + 1].split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                            window_size = Some([w, h]);
                        }
                    }
                    i += 1;
                }
            }
            "--find" => {
                if i + 1 < args.len() {
                    start_find = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--deep" => start_deep = true,
            "--guide" => start_guide = true,
            "--menu" => start_menu = true,
            // Opens the "Other Application…" chooser on the first frame, with
            // this file, so the chooser can be photographed without a mouse.
            "--open-with" => {
                if i + 1 < args.len() {
                    open_with_file = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            // Moves a folder's contents to the Trash once so the Undo toast can be
            // photographed; see the comment on `pending_trash_demo`.
            "--trash-demo" => {
                if i + 1 < args.len() {
                    trash_demo = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--keep-settings" => {
                keep_settings = true;
            }
            "--pictures" | "--gallery" => start_pictures = true,
            "--below" => start_below = true,
            "--dims" => start_dims = true,
            "--details" => start_details = true,
            "--play" | "--slideshow" => play = true,
            "--compare" => {
                start_compare = Some(match args.get(i + 1).map(|s| s.as_str()) {
                    Some("split") => gallery::CompareMode::Split,
                    Some("overlay") => gallery::CompareMode::Overlay,
                    Some("side") => gallery::CompareMode::Side,
                    _ => gallery::CompareMode::Side,
                });
                if matches!(
                    args.get(i + 1).map(|s| s.as_str()),
                    Some("split") | Some("overlay") | Some("side")
                ) {
                    i += 1;
                }
            }
            "--thumbs" => {
                if i + 1 < args.len() {
                    start_thumbs = args[i + 1].parse::<f32>().ok();
                    i += 1;
                }
            }
            "--viewer" => {
                if i + 1 < args.len() {
                    start_viewer = Some((PathBuf::from(&args[i + 1]), play));
                    i += 1;
                }
            }
            other => {
                if start_dir.is_none() {
                    let p = PathBuf::from(other);
                    if p.is_dir() {
                        start_dir = Some(p);
                    }
                }
            }
        }
        i += 1;
    }

    // The window opens the size it was left, as the original's saved window state
    // does. A capture keeps the fixed size so the picture is the same every run,
    // and `--size` still wins.
    let startup_size = window_size.or_else(|| {
        if screenshot.is_some() {
            return None;
        }
        Config::load()
            .window_px
            .filter(|s| s[0] >= 640.0 && s[1] >= 400.0)
    });
    // The window also opens where it was left. A capture is taken at a fixed
    // size and place so the picture is the same every run.
    let startup_pos = if screenshot.is_some() {
        None
    } else {
        startup_position(&Config::load())
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(startup_size.unwrap_or([1440.0, 900.0]))
        .with_min_inner_size([880.0, 540.0])
        // The window title is the name the person sees on the title bar and
        // in the task switcher. Renamed from the original program's name to
        // this build's own name, as asked.
        .with_title(APP_NAME)
        .with_app_id("dolphincraft");
    if let Some(pos) = startup_pos {
        viewport = viewport.with_position(egui::Pos2::new(pos[0], pos[1]));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let shot = screenshot.clone();
    let viewer_arg = start_viewer.clone().map(|(path, _)| (path, play));
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut app = App::new(
                cc,
                start_dir.clone(),
                shot.clone(),
                start_zoom,
                start_view,
                keep_settings,
                viewer_arg.clone(),
                start_pictures,
                start_thumbs,
                start_compare,
                start_dims,
                start_find,
                start_deep,
                start_guide,
                start_menu,
                start_details,
                start_below,
            );
            app.pending_trash_demo = trash_demo.clone();
            app.pending_open_with = open_with_file.clone();
            Ok(Box::new(app))
        }),
    )
}
