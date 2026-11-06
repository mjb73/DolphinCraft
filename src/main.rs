//! Dolphin-style file manager: window, toolbar, tabs, places, panes, status.

/// The name on the window title bar, in the task switcher and in the app id.
/// The original program declares its own name "Dolphin" (`@title` of its
/// KAboutData block at 0x12d350); this build carries its own name instead, as
/// the person asked. Everything that shows the app's name reads this one string.
pub const APP_NAME: &str = "DolphinCraft";

mod config;
mod foldercolor;
mod fsutil;
mod panels;
mod search;
mod selftest;
mod state;
mod theme;
mod views;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Modifiers, Pos2, Rect,
    Sense, Stroke, Vec2, ViewportCommand,
};

use config::Config;
use search::{DupScan, ThumbCache};
use state::{Clipboard, Pane, Request, ViewMode, TRASH_URI};
use theme::*;
use views::ViewCtx;

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
/// and the nine item gaps between the ten things in the cluster.
pub const TOOLBAR_CLUSTER_FIXED: f32 =
    7.0 * TOOLBAR_ICON_W + 2.0 * (4.0 + TOOLBAR_ITEM_GAP) + 9.0 * TOOLBAR_ITEM_GAP;

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
    pub cluster_w: f32,
    pub bar_w: f32,
}

/// Split `avail` pixels between the location bar and the cluster. `label_w` is
/// the measured width of the sort label's text. The invariant this function
/// keeps is `bar_w + TOOLBAR_CLUSTER_GAP + cluster_w <= avail`, so the two can
/// never be drawn over each other.
pub fn toolbar_metrics(avail: f32, label_w: f32, show_bar: bool) -> ToolbarMetrics {
    // The sort button carries the label plus the style's 7 px side padding.
    let with_label = TOOLBAR_CLUSTER_FIXED + label_w + 2.0 * 7.0;
    let with_icon = TOOLBAR_CLUSTER_FIXED + TOOLBAR_ICON_W;
    let without = with_icon - TOOLBAR_ICON_W - TOOLBAR_ITEM_GAP;

    let mut sort = SortControl::Label;
    let mut cluster_w = with_label;
    if show_bar {
        if avail - with_label - TOOLBAR_CLUSTER_GAP < TOOLBAR_BAR_MIN {
            sort = SortControl::Icon;
            cluster_w = with_icon;
        }
        if avail - with_icon - TOOLBAR_CLUSTER_GAP < TOOLBAR_BAR_MIN {
            sort = SortControl::Hidden;
            cluster_w = without;
        }
    }
    let bar_w = if show_bar {
        (avail - cluster_w - TOOLBAR_CLUSTER_GAP).max(0.0)
    } else {
        0.0
    };
    ToolbarMetrics {
        sort,
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
    pub shortcut_edit: Option<ShortcutEdit>,
    pub toast: Option<(String, Instant)>,
    pub folder_sizes: HashMap<PathBuf, u64>,
    pub size_rx: Receiver<(PathBuf, u64)>,
    pub size_tx: Sender<(PathBuf, u64)>,
    pub shot: Option<Shot>,
    pub frames: u64,
    pub config_dirty: bool,
    pub next_pane_id: usize,
    pub split_frac: f32,
    pub search_focus: bool,
    pub filter_focus: bool,
    /// Which of the person's own shortcut rows is being dragged, if any.
    /// The value is the index into `config.bookmarks`.
    pub shortcut_drag: Option<usize>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        start_dir: Option<PathBuf>,
        shot: Option<PathBuf>,
        start_zoom: Option<u8>,
        start_view: Option<usize>,
        keep_settings: bool,
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
        let mut pane = Pane::new(next_id, dir);
        next_id += 1;
        pane.view = ViewMode::from_index(cfg.view);
        pane.zoom = cfg.zoom;
        if let Some(v) = start_view {
            pane.view = ViewMode::from_index(v.min(2));
        }
        if let Some(z) = start_zoom {
            let slot = pane.view.index();
            pane.zoom[slot] = z.min(16);
        }
        pane.show_hidden = cfg.show_hidden;
        pane.previews = cfg.previews;
        pane.folders_first = cfg.folders_first;
        if cfg.show_hidden {
            pane.reload();
        }

        let shot_mode = shot.is_some();
        let (size_tx, size_rx) = channel();
        let mut app = App {
            tabs: vec![Tab {
                panes: vec![pane],
                active: 0,
            }],
            active_tab: 0,
            split: cfg.split,
            places_visible: cfg.places_visible,
            info_panel: cfg.info_panel,
            clipboard: Clipboard::new(),
            thumbs: ThumbCache::new(),
            config: cfg,
            reqs: Vec::new(),
            confirm: None,
            props: None,
            bulk: None,
            dups: None,
            quick: None,
            new_folder: None,
            shortcut_edit: None,
            toast: None,
            folder_sizes: HashMap::new(),
            size_rx,
            size_tx,
            shot: shot.map(|p| Shot {
                path: p,
                requested: false,
            }),
            frames: 0,
            config_dirty: false,
            next_pane_id: next_id,
            split_frac: 0.5,
            search_focus: false,
            filter_focus: false,
            shortcut_drag: None,
        };
        // The remembered split view needs its second pane rebuilt, otherwise a
        // saved "split" flag with a single pane hides the location bar entirely
        // (the toolbar gives that slot to the panes). Screenshot mode stays a
        // plain single view so the capture is the same every time.
        if app.split && !shot_mode {
            app.ensure_split();
        } else if shot_mode {
            app.split = false;
        }
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
                    let metrics = toolbar_metrics(avail, label_w, show_bar);
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
                    let cluster_inner = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let menu = toolbar_button(ui, "menu", true);
                        menu.context_menu(|ui| self.app_menu(ui));

                        if toolbar_button(ui, "search", true)
                            .on_hover_text("Search for files and folders (Ctrl+F)")
                            .clicked()
                        {
                            let pane = self.pane_mut();
                            pane.filter_on = true;
                            self.search_focus = true;
                        }
                        let filter_on = self.pane().filter_on;
                        if toolbar_button_active(ui, "filter", filter_on)
                            .on_hover_text("Show Filter Bar (Ctrl+I)")
                            .clicked()
                        {
                            let pane = self.pane_mut();
                            pane.filter_on = !pane.filter_on;
                            if !pane.filter_on {
                                pane.filter.clear();
                                pane.search = None;
                            } else {
                                self.filter_focus = true;
                            }
                        }
                        if toolbar_button_active(ui, "split", self.split)
                            .on_hover_text("Split view (F3)")
                            .clicked()
                        {
                            if self.split {
                                self.unsplit();
                            } else {
                                self.ensure_split();
                            }
                            self.config_dirty = true;
                        }
                        ui.add_space(4.0);
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
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| self.sort_menu(ui));
        resp
    }

    fn sort_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(190.0);
        let pane = self.pane_mut();
        for key in [
            state::SortKey::Name,
            state::SortKey::Size,
            state::SortKey::Type,
            state::SortKey::Modified,
        ] {
            let checked = pane.sort == key;
            if ui.selectable_label(checked, key.label()).clicked() {
                if pane.sort == key {
                    pane.asc = !pane.asc;
                } else {
                    pane.sort = key;
                    pane.asc = true;
                }
                pane.sort_entries();
                ui.close();
            }
        }
        ui.separator();
        if ui
            .checkbox(&mut pane.folders_first, "Show folders first")
            .changed()
        {
            pane.sort_entries();
        }
        if ui
            .checkbox(&mut pane.show_hidden, "Show hidden files")
            .changed()
        {
            pane.reload();
        }
        if ui.checkbox(&mut pane.previews, "Show preview").changed() {}
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
            });

        if let Some(p) = navigate {
            self.pane_mut().navigate(p);
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
                ui.horizontal_centered(|ui| {
                    let pane = self.pane();
                    let sel_count = pane.sel.len();
                    let shown = pane.display_entries();
                    let mut text = if pane.search.is_some() {
                        let s = pane.search.as_ref().unwrap();
                        if !s.done {
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

                    let view_idx = self.pane().view.index();
                    let mut zoom_value = self.pane().zoom[view_idx] as f32;
                    let icon = self.pane().icon_size() as u32;
                    let mut zoom_changed = false;

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, "+").on_hover_text("Zoom in").clicked() {
                            self.pane_mut().zoom_by(1);
                            zoom_changed = true;
                        }
                        let slider = ui.add_sized(
                            Vec2::new(110.0, 14.0),
                            egui::Slider::new(&mut zoom_value, 0.0..=16.0)
                                .show_value(false)
                                .step_by(1.0),
                        );
                        if slider.changed() {
                            let z = zoom_value.round() as u8;
                            let vi = self.pane().view.index();
                            self.pane_mut().zoom[vi] = z;
                            zoom_changed = true;
                        }
                        if small_button(ui, "−").on_hover_text("Zoom out").clicked() {
                            self.pane_mut().zoom_by(-1);
                            zoom_changed = true;
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
                Request::Trash(paths) => {
                    if !paths.is_empty() {
                        self.confirm = Some(Confirm {
                            title: "Move to Trash".to_string(),
                            body: format!(
                                "Move {} item{} to the Trash?",
                                paths.len(),
                                if paths.len() == 1 { "" } else { "s" }
                            ),
                            action: ConfirmAction::Trash(paths),
                        });
                    }
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
                Request::DeletePermanent(paths) => {
                    if !paths.is_empty() {
                        self.confirm = Some(Confirm {
                            title: "Delete Permanently".to_string(),
                            body: format!(
                                "Delete {} item{} permanently? This cannot be undone.",
                                paths.len(),
                                if paths.len() == 1 { "" } else { "s" }
                            ),
                            action: ConfirmAction::Delete(paths),
                        });
                    }
                }
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
                    for p in paths {
                        let tx = self.size_tx.clone();
                        std::thread::spawn(move || {
                            let size = search::dir_size(&p);
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
                    self.config_dirty = true;
                }
                Request::Toast(msg) => self.toast(&msg),
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
        let typing = ctx.wants_keyboard_input();
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            if self.quick.is_some() {
                self.quick = None;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.bulk.is_some() {
                self.bulk = None;
            } else if self.dups.is_some() {
                self.dups = None;
            } else if self.new_folder.is_some() {
                self.new_folder = None;
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
                reqs.push(Request::Toast("__newtab__".into()));
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
                self.pane_mut().filter_on = true;
                self.search_focus = true;
            }
            "__filter__" => {
                let on = !self.pane().filter_on;
                let pane = self.pane_mut();
                pane.filter_on = on;
                if !on {
                    pane.filter.clear();
                    pane.search = None;
                }
                self.filter_focus = on;
            }
            "__selectall__" => self.pane_mut().select_all(),
            "__zoomin__" => {
                self.pane_mut().zoom_by(1);
                self.config.zoom = self.pane().zoom;
            }
            "__zoomout__" => {
                self.pane_mut().zoom_by(-1);
                self.config.zoom = self.pane().zoom;
            }
            "__zoomreset__" => {
                self.pane_mut().zoom = [4, 0, 0];
                self.config.zoom = self.pane().zoom;
            }
            "__back__" => self.pane_mut().go_back(),
            "__forward__" => self.pane_mut().go_forward(),
            "__up__" => self.pane_mut().go_up(),
            "__refresh__" => self.pane_mut().reload(),
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
        let count = self.pane().display_entries().len();
        if count == 0 {
            return;
        }
        let cur = self.pane().cursor as i32 + delta;
        let next = cur.clamp(0, count as i32 - 1) as usize;
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
        self.handle_shortcuts(ctx);
        self.top_toolbar(ctx);
        self.tab_strip(ctx);
        self.places_panel(ctx);
        self.info_panel_ui(ctx);
        self.status_bar(ctx);
        self.central(ctx);
        self.handle_requests(ctx);
        self.toast_ui(ctx);
        self.dialogs(ctx);
        self.thumbs.poll(ctx);

        // A captured picture must not rewrite the person's settings.
        if self.config_dirty && self.shot.is_none() {
            let mut cfg = self.config.clone();
            cfg.split = self.split;
            cfg.places_visible = self.places_visible;
            cfg.info_panel = self.info_panel;
            cfg.show_hidden = self.pane().show_hidden;
            cfg.previews = self.pane().previews;
            cfg.view = self.pane().view.index();
            cfg.zoom = self.pane().zoom;
            cfg.folders_first = self.pane().folders_first;
            cfg.last_dir = self.pane().current_dir().display().to_string();
            cfg.save();
            self.config = cfg;
            self.config_dirty = false;
        }
        self.screenshot_tick(ctx);
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
            "--keep-settings" => {
                keep_settings = true;
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([880.0, 540.0])
            // The window title is the name the person sees on the title bar and
            // in the task switcher. Renamed from the original program's name to
            // this build's own name, as asked.
            .with_title(APP_NAME)
            .with_app_id("dolphincraft"),
        ..Default::default()
    };
    let shot = screenshot.clone();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                cc,
                start_dir.clone(),
                shot.clone(),
                start_zoom,
                start_view,
                keep_settings,
            )))
        }),
    )
}
