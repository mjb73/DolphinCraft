//! Drawing a folder view: breadcrumb, Details/Icons/Compact layouts,
//! selection, inline rename and context menus.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, Response, Sense,
    Stroke, TextStyle, UiBuilder, Vec2,
};

use crate::fsutil::{self, Kind};
use crate::search::ThumbCache;
use crate::state::{Clipboard, Pane, Request, SortKey, ViewMode};
use crate::theme;
use crate::NewKind;

/// Room kept for the "…" that stands in for the elided start of a path.
const LEADER_W: f32 = 12.0;

pub struct ViewCtx<'a> {
    pub thumbs: &'a mut ThumbCache,
    pub clipboard: &'a Clipboard,
    pub reqs: &'a mut Vec<Request>,
    pub split: bool,
    pub active: bool,
    pub show_location_bar: bool,
    /// The folder the other view shows, when there is one: the destination the
    /// two *Other View* rows copy or move into.
    pub other_dir: Option<std::path::PathBuf>,
}

/// The column rectangles of the details view, in drawing order. `path` is
/// present only while a search is showing (see `Pane::column_rects`).
pub struct DetailsCols {
    pub name: Rect,
    pub path: Option<Rect>,
    pub size: Rect,
    pub date: Rect,
}

/// Where the find bar's controls ended up this frame: the close button and the
/// cluster holding the two scope chips. Returned so a check can prove the close
/// button is inside the bar and clear of the chips — it used to be drawn last,
/// after the field had already taken the room it needed, and fell off the edge.
#[derive(Clone, Copy)]
pub struct FindBarLayout {
    pub close: Rect,
    pub chips: Rect,
}

impl Default for FindBarLayout {
    fn default() -> Self {
        Self {
            close: Rect::NOTHING,
            chips: Rect::NOTHING,
        }
    }
}

/// A small square button whose picture is one of the theme's drawn glyphs. The
/// glyphs are line shapes rather than characters, so no font is ever missing a
/// symbol — the `✕` character in particular is not in this app's font and drew
/// as an empty box where the find bar's close button should have been.
fn glyph_button(ui: &mut egui::Ui, glyph: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(24.0, 22.0), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(1.0, 1.0)),
            CornerRadius::same(3),
            theme::row_hover(),
        );
    }
    theme::draw_glyph(
        ui.painter(),
        rect,
        glyph,
        if hovered { theme::text() } else { theme::text_dim() },
    );
    resp.on_hover_text(tip)
}

/// The Size cell of one details row: a folder says how many items it holds, a
/// file its size, and a count or size that was not read is a dash. Shared by the
/// row that draws it and the measurement that sizes the column.
fn size_label(entry: &fsutil::Entry) -> String {
    if entry.is_dir {
        match entry.children {
            Some(n) => format!("{} item{}", n, if n == 1 { "" } else { "s" }),
            None => "—".to_string(),
        }
    } else {
        fsutil::format_size(entry.size)
    }
}

/// Where a search hit sits, as the column writes it: the folder relative to the
/// folder the search started from, so `2024/03` rather than the same long prefix
/// on every row. `None` when the file lies directly in the searched folder —
/// there is no folder of its own to name. A hit outside the searched folder (a
/// link followed elsewhere) keeps its own full path.
pub(crate) fn hit_folder(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let parent = path.parent()?;
    match parent.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => None,
        Ok(rel) => Some(rel.display().to_string()),
        Err(_) => Some(parent.display().to_string()),
    }
}

/// The colour a folder was given, ready for drawing. Files never take one.
fn tint_of(entry: &fsutil::Entry) -> Option<Color32> {
    if !entry.is_dir {
        return None;
    }
    entry.tint.map(|c| theme::c(c[0], c[1], c[2]))
}

/// Draw a preview texture inside `rect` the way the original fits a preview
/// pixmap into an item's icon area: the aspect ratio is kept, the picture is
/// centred and it fills as much of the square as it can, whether that scales it
/// up or down (`KPixmapModifier::scale` @0x15e820, called from
/// `KStandardItemListWidget::drawPixmap` @0x135bb0).
fn draw_preview(painter: &egui::Painter, tex: &egui::TextureHandle, rect: Rect) -> Option<Rect> {
    let src = tex.size();
    if src[0] == 0 || src[1] == 0 || rect.width() <= 0.0 {
        return None;
    }
    let scale = (rect.width() / src[0] as f32).min(rect.height() / src[1] as f32);
    let size = Vec2::new(src[0] as f32 * scale, src[1] as f32 * scale);
    let target = Rect::from_center_size(rect.center(), size);
    painter.image(
        tex.id(),
        target,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    Some(target)
}

/// Corner radius for a thumbnail drawn at `icon` px. The gallery's own frame
/// uses 10 px at its 400 px ceiling (`PICTURE_CORNER`); this keeps the same
/// look at smaller sizes instead of rounding a 32 px icon into a circle.
fn picture_radius(icon: f32) -> f32 {
    (icon * 0.035).clamp(3.0, crate::gallery::PICTURE_CORNER as f32)
}

impl Pane {
    pub fn ui(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let rect = ui.available_rect_before_wrap();
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect);

        if vctx.show_location_bar {
            egui::Frame::new()
                .fill(theme::header_bg())
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(&mut child, |ui| {
                    egui::Frame::new()
                        .fill(theme::header_bg())
                        .stroke(egui::Stroke::new(1.0, theme::border()))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(7, 2))
                        .show(ui, |ui| {
                            ui.set_min_width((ui.available_width() - 4.0).max(60.0));
                            ui.set_min_height(22.0);
                            self.breadcrumb_ui(ui, vctx.reqs);
                        });
                });
        }
        self.find_bar(&mut child, vctx);

        // The active directory itself: a right-click on the space between the
        // items — or on an empty folder, where there are no items at all —
        // offers the original's *Create New* menu. The press lands here only
        // when no row is under it: a row registers its own menu later, and egui
        // gives a click to the topmost widget that can take it.
        let in_trash = self.is_trash();
        let dir = self.current_dir();
        let bg_rect = child.available_rect_before_wrap();
        if !in_trash && bg_rect.width() > 8.0 && bg_rect.height() > 8.0 {
            let bg = child.interact(bg_rect, Id::new(("dir_background", self.id)), Sense::click());
            bg.context_menu(|ui| {
                ui.set_min_width(200.0);
                ui.menu_button("Create New", |ui| {
                    ui.set_min_width(220.0);
                    if crate::menu_row(ui, "Folder…", "F10").clicked() {
                        vctx.reqs.push(Request::NewItem(NewKind::Folder, dir.clone()));
                        ui.close();
                    }
                    if crate::menu_row(ui, "Text File…", "").clicked() {
                        vctx.reqs.push(Request::NewItem(NewKind::TextFile, dir.clone()));
                        ui.close();
                    }
                });
            });
        }

        // Ctrl + wheel zooms the view, the same one-step-per-notch change the
        // +/- buttons in the status bar make.
        if self.wheel_zoom_ui(&child) {
            vctx.reqs.push(Request::ZoomChanged);
        }

        if self.pictures {
            // The gallery replaces whichever of the three layouts is underneath,
            // so turning it off puts the folder back exactly as it was.
            self.ui_pictures(&mut child, vctx);
        } else {
            match self.view {
                ViewMode::Details => self.ui_details(&mut child, vctx),
                ViewMode::Icons => self.ui_icons(&mut child, vctx),
                ViewMode::Compact => self.ui_compact(&mut child, vctx),
            }
        }

        // Active view marker: a thin line along the top of the focused view.
        if vctx.split && vctx.active {
            let top = rect.top();
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left(), top),
                    Pos2::new(rect.right(), top),
                ],
                Stroke::new(2.0, theme::accent()),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Ctrl + wheel zoom
    // -----------------------------------------------------------------------

    /// Zoom one step per wheel notch while Ctrl is held, wherever the pointer
    /// rests inside this view. Measured from `DolphinView::wheelEvent`
    /// @0x272710 in libdolphinprivate.so.6: without the Control modifier the
    /// event is ignored (the view scrolls instead), with it the wheel delta is
    /// added up and every 120 units (`0x78`) calls `slotIncreaseZoom` /
    /// `slotDecreaseZoom`, i.e. `setZoomLevel(level ± 1)` — the very same step
    /// the status bar's + and − buttons take.
    ///
    /// Returns true when the zoom level actually changed.
    fn wheel_zoom_ui(&mut self, ui: &egui::Ui) -> bool {
        let area = ui.available_rect_before_wrap();
        if !ui.rect_contains_pointer(area) {
            return false;
        }
        let units = ui.input(|i| {
            let mut units = 0.0f32;
            for ev in &i.events {
                if let egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } = ev
                {
                    // The original tests the Control bit alone, so Ctrl+Shift or
                    // Ctrl+Alt still zoom.
                    if !(modifiers.ctrl || modifiers.command) {
                        continue;
                    }
                    if delta.y == 0.0 {
                        continue;
                    }
                    units += crate::state::wheel_units(*unit, delta.y);
                }
            }
            units
        });
        if units == 0.0 {
            return false;
        }
        let steps = crate::state::wheel_zoom_steps(&mut self.wheel_acc, units);
        if steps == 0 {
            return false;
        }
        let before = self.zoom_level();
        self.zoom_by(steps);
        self.zoom_level() != before
    }

    // -----------------------------------------------------------------------
    // Location bar
    // -----------------------------------------------------------------------

    /// Turn the location bar into a text field with the whole address
    /// selected, so typing or pasting replaces it in one go. The original's own
    /// help text: "You can also switch to editing by clicking to the right of
    /// the location and switch back by confirming the edited location."
    pub fn begin_location_edit(&mut self) {
        let current = if self.is_trash() {
            String::new()
        } else {
            self.display_path()
        };
        self.location_edit = Some(current);
        self.location_focus = false;
        self.location_select = true;
    }

    pub fn breadcrumb_ui(&mut self, ui: &mut egui::Ui, reqs: &mut Vec<Request>) {
        if self.location_edit.is_some() {
            let field_id = ui.make_persistent_id("location-field");
            let mut submitted: Option<String> = None;
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                if let Some(buf) = self.location_edit.as_mut() {
                    let resp = ui.add_sized(
                        Vec2::new((ui.available_width() - 8.0).max(80.0), 22.0),
                        egui::TextEdit::singleline(buf)
                            .id(field_id)
                            .font(TextStyle::Body)
                            .margin(egui::Margin::symmetric(4, 2))
                            .hint_text("Type or paste a location, then press Enter"),
                    );
                    if !self.location_focus {
                        resp.request_focus();
                        self.location_focus = true;
                    }
                    if self.location_select {
                        // Select everything, so a paste replaces the path
                        // instead of landing at the end of it.
                        if let Some(mut state) =
                            egui::text_edit::TextEditState::load(ui.ctx(), field_id)
                        {
                            let end = buf.chars().count();
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(end),
                            )));
                            state.store(ui.ctx(), field_id);
                        }
                        self.location_select = false;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submitted = Some(buf.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                }
            });
            if let Some(raw) = submitted {
                self.location_edit = None;
                self.location_focus = false;
                self.location_select = false;
                // A folder opens; a file opens its folder with the file
                // selected; anything else keeps the field open and says so.
                if let Some(message) = self.navigate_input(&raw) {
                    reqs.push(Request::Toast(message));
                    self.location_edit = Some(raw);
                    self.location_select = true;
                }
            } else if cancel {
                self.location_edit = None;
                self.location_focus = false;
                self.location_select = false;
            }
            return;
        }

        // The bar as a whole answers to a click: in the original a click that
        // does not land on a breadcrumb switches the bar to editing.
        let bar = ui
            .interact(
                ui.max_rect(),
                ui.id().with("location-bar"),
                Sense::click(),
            )
            .on_hover_text("Click to type or paste a location (Ctrl+L)");
        let mut crumb_clicked = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let crumbs = self.breadcrumbs();
            let last = crumbs.len().saturating_sub(1);
            // Breadcrumbs give way from the left when the bar is too narrow for
            // the whole path, as the original's bar does: the current folder
            // stays, the beginning collapses into a "…". Without this the row
            // simply grew wider than the bar it sits in.
            let font = FontId::proportional(13.0);
            let widths: Vec<f32> = crumbs
                .iter()
                .map(|(label, _)| {
                    ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), theme::text()).size().x)
                        + 8.0
                })
                .collect();
            let sep = 10.0;
            let avail = (ui.available_width() - LEADER_W - 2.0).max(0.0);
            let mut start = 0usize;
            let mut total: f32 =
                widths.iter().sum::<f32>() + sep * crumbs.len().saturating_sub(1) as f32;
            while total > avail && start + 1 < crumbs.len() {
                total -= widths[start] + sep;
                start += 1;
            }
            if start > 0 {
                ui.add_space(1.0);
                ui.label(
                    egui::RichText::new("…")
                        .color(theme::text_faint())
                        .size(13.0),
                )
                .on_hover_text("Earlier folders in this path");
            }
            for (i, (label, path)) in crumbs.iter().enumerate().skip(start) {
                let is_current = i == last;
                // Whatever room is left at this point: the last segment (and any
                // segment after a long path) is elided to it rather than drawn
                // past the bar.
                let left = ui.available_width() - 12.0;
                let shown = theme::elide(ui, label, left.max(24.0), font.clone());
                let label = &shown;
                let text = egui::RichText::new(label).size(13.0);
                let text = if is_current { text.strong() } else { text };
                let btn = egui::Button::new(text)
                    .fill(Color32::TRANSPARENT)
                    .corner_radius(CornerRadius::same(3))
                    .min_size(Vec2::new(0.0, 22.0));
                let resp = ui.add(btn);
                if resp.clicked() {
                    crumb_clicked = true;
                    reqs.push(Request::Navigate(path.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Copy Location").clicked() {
                        ui.ctx().copy_text(path.display().to_string());
                        ui.close();
                    }
                    if ui.button("Open in New Tab").clicked() {
                        reqs.push(Request::OpenInNewTab(path.clone()));
                        ui.close();
                    }
                    if ui.button("Open in New Window").clicked() {
                        crate::spawn_folder_window(&path);
                        ui.close();
                    }
                });
                if i != last {
                    ui.label(egui::RichText::new("›").color(theme::text_faint()).size(12.0));
                }
            }
            // The empty stretch after the last breadcrumb: this is the part of
            // the original's bar that turns into a text field when clicked.
            let rest = ui.available_width();
            if rest > 4.0 {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(rest, 22.0), Sense::hover());
                if resp.hovered() && !bar.hovered() {
                    ui.painter().rect_filled(
                        rect.shrink(1.0),
                        CornerRadius::same(3),
                        theme::row_hover(),
                    );
                }
            }
        });
        if bar.clicked() && !crumb_clicked {
            self.begin_location_edit();
        }
    }

    // -----------------------------------------------------------------------
    // Find bar
    // -----------------------------------------------------------------------

    /// The one place a name is looked for. Typing narrows the folder as you
    /// type; the two chips say exactly how far the looking goes, so the person
    /// never has to know that pressing Enter secretly turns a filter into a
    /// search. The original keeps both on one bar too ("Filter" /
    /// "Toggle Search Bar", `dolphin` .ui strings), but hides the choice
    /// behind the keystroke.
    pub(crate) fn find_bar(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) -> FindBarLayout {
        let mut drawn = FindBarLayout::default();
        if !self.filter_on {
            return drawn;
        }
        let mut stop = false;
        let mut close = false;
        let mut submit = false;
        let mut scope: Option<bool> = None;
        egui::Frame::new()
            .fill(theme::header_bg())
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let searching = self.search.is_some();
                    let deep = self.find_deep;
                    let found = self.search.as_ref().map(|s| s.results.len()).unwrap_or(0);
                    // The right-hand cluster — the close button, and while a
                    // search is running the count and Stop — is placed against
                    // the right edge *first*, in a child of its own, so the
                    // field and the chips can only take room this cluster did
                    // not need. Placing it last is what pushed the close button
                    // out past the edge of the window: the field had already
                    // eaten the room it was meant to sit in.
                    let row = ui.available_rect_before_wrap();
                    let tail_left = {
                        let mut tail = ui.new_child(
                            UiBuilder::new()
                                .max_rect(row)
                                .layout(Layout::right_to_left(Align::Center)),
                        );
                        let close_resp = glyph_button(&mut tail, "close", "Close the find bar (Esc)");
                        drawn.close = close_resp.rect;
                        if close_resp.clicked() {
                            close = true;
                        }
                        if searching {
                            if tail.add(egui::Button::new("Stop").small()).clicked() {
                                stop = true;
                            }
                            tail.add_space(4.0);
                            tail.label(
                                egui::RichText::new(format!("{found} found"))
                                    .size(11.0)
                                    .color(theme::text_faint()),
                            );
                        }
                        tail.min_rect().left()
                    };
                    let gap = ui.spacing().item_spacing.x;
                    // The two scope chips go in their own child as well, hung off
                    // the left of that cluster, so the field can be sized from
                    // where they really ended rather than from an estimate of
                    // how wide a chip is.
                    let chips_rect = Rect::from_min_max(
                        row.min,
                        Pos2::new(
                            (tail_left - gap * 2.0).max(row.left()),
                            row.bottom(),
                        ),
                    );
                    let mut chips = ui.new_child(
                        UiBuilder::new()
                            .max_rect(chips_rect)
                            .layout(Layout::right_to_left(Align::Center)),
                    );
                    // Right-to-left, so "In subfolders" is drawn first and lands
                    // right of "This folder", which is how the pair reads.
                    if find_chip(&mut chips, "In subfolders", deep)
                        .on_hover_text("Look through every folder below this one as well")
                        .clicked()
                    {
                        scope = Some(true);
                    }
                    if find_chip(&mut chips, "This folder", !deep)
                        .on_hover_text("Show only the items in this folder whose name matches")
                        .clicked()
                    {
                        scope = Some(false);
                    }
                    drawn.chips = chips.min_rect();
                    let chips_left = chips.min_rect().left();
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    theme::draw_glyph(ui.painter(), icon_rect, "search", theme::text_dim());
                    ui.add_space(4.0);
                    // The field takes what the chips and that cluster leave, so
                    // nothing is ever drawn past the right edge.
                    let w = (chips_left - gap - ui.cursor().left()).max(80.0);
                    let hint = if searching {
                        "Looking in this folder and below…"
                    } else if deep {
                        "Type to search this folder and everything below"
                    } else {
                        "Type to show only matching names in this folder"
                    };
                    let resp = ui.add_sized(
                        Vec2::new(w, 22.0),
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(hint)
                            .font(TextStyle::Body)
                            .margin(egui::Margin::symmetric(5, 2)),
                    );
                    if !self.filter_focus {
                        resp.request_focus();
                        self.filter_focus = true;
                    }
                    if resp.changed() {
                        self.sel.clear();
                        self.cursor = 0;
                        self.refilter();
                        if self.find_deep {
                            if self.filter.trim().is_empty() {
                                self.search = None;
                            } else {
                                self.start_search();
                            }
                        }
                    }
                    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });
        if let Some(deep) = scope {
            self.set_find_scope(deep);
            self.filter_focus = true;
        }
        if submit {
            if !self.find_deep {
                self.set_find_scope(true);
            }
            if !self.filter.trim().is_empty() {
                self.start_search();
            }
            self.filter_focus = true;
        }
        if stop {
            self.search = None;
            self.filter_focus = true;
        }
        if close {
            self.filter_on = false;
            self.filter.clear();
            self.search = None;
            self.filter_focus = false;
            vctx.reqs.push(Request::Navigate(self.path.clone()));
        }
        drawn
    }

    // -----------------------------------------------------------------------
    // Details view
    // -----------------------------------------------------------------------

    /// The column rectangles of the details view for a row (or the header
    /// row), inset by the original's 20 px side padding — see
    /// `theme::DETAILS_SIDE_PAD` and `DetailsModeSettings` @0x2b0f10.
    pub fn details_columns(&self, rect: Rect) -> DetailsCols {
        self.column_rects(rect.shrink2(Vec2::new(theme::DETAILS_SIDE_PAD, 0.0)))
    }

    fn column_rects(&self, rect: Rect) -> DetailsCols {
        let w = rect.width();
        let mut date_w = (w * 0.28).clamp(64.0, 150.0);
        let mut size_w = (w * 0.16).clamp(44.0, 88.0);
        // Only when the walk covers the folders *below* this one: a "This
        // folder" search has every row in the folder the bar already names, so
        // the column would hold one value repeated down the list. It is also
        // absent when every hit lies in the searched folder itself.
        let show_path = self.find_deep && self.search.is_some() && self.search_path_px > 0.0;
        // Both text columns are sized from the widest string each has to hold,
        // measured with the fonts the rows are drawn with
        // (`measure_search_columns`), so a name is never cut short while the
        // folder beside it has room to spare. The icon sits inside the name
        // column, and each label keeps a few pixels of air at its right edge.
        let icon_w = self.icon_size().clamp(16.0, theme::MAX_ICON_PX) + 8.0;
        let name_need = icon_w + self.search_name_px + 8.0;
        let path_need = self.search_path_px + 10.0;
        if show_path {
            // The original's 16 % / 28 % for Size and Modified are what a plain
            // folder shows, and they stay while there is room. When the two text
            // columns cannot both fit in what those leave, the numeric columns
            // give their slack back — down to what their own cells hold — rather
            // than have a name or a folder elided for the sake of empty space.
            if name_need + path_need > w - size_w - date_w {
                date_w = (self.search_date_px + 16.0).max(64.0).min(date_w);
                size_w = (self.search_size_px + 14.0).max(44.0).min(size_w);
            }
        }
        // What the two text columns share once size and date have theirs.
        let text_w = w - size_w - date_w;
        let mut name_w = text_w.max(w * 0.45);
        let mut path = None;
        let mut path_w = 0.0;
        if show_path {
            let need = name_need + path_need;
            if need <= text_w {
                let slack = (text_w - need) * 0.5;
                name_w = name_need + slack;
                path_w = path_need + slack;
            } else {
                // The row physically cannot hold both. The name keeps its room
                // whenever that still leaves the folder a usable 90 px, because
                // the name is what the row is about; below that neither can be
                // shown whole and the name takes the larger half.
                name_w = if text_w - name_need >= 90.0 {
                    name_need
                } else {
                    text_w * 0.55
                };
                path_w = text_w - name_w;
            }
            path = Some(Rect::from_min_max(
                Pos2::new(rect.left() + name_w, rect.top()),
                Pos2::new(rect.left() + name_w + path_w, rect.bottom()),
            ));
        }
        let name = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + name_w, rect.bottom()),
        );
        let after_name = rect.left() + name_w + path_w;
        let size = Rect::from_min_max(
            Pos2::new(after_name, rect.top()),
            Pos2::new(after_name + size_w, rect.bottom()),
        );
        let date = Rect::from_min_max(
            Pos2::new(rect.right() - date_w, rect.top()),
            rect.max,
        );
        DetailsCols { name, path, size, date }
    }

    /// Measure the widest name and the widest folder in the search results, with
    /// the very fonts the details rows are drawn with, so the two text columns
    /// can be sized to what they have to hold instead of to a share of the row.
    /// Only entries that arrived since the last frame are measured, and a new
    /// search starts the count again (`Pane::start_search`).
    pub fn measure_search_columns(&mut self, ui: &egui::Ui) {
        let Some(total) = self.search.as_ref().map(|s| s.results.len()) else {
            self.search_name_px = 0.0;
            self.search_path_px = 0.0;
            self.search_measured = 0;
            return;
        };
        if !self.find_deep || self.search_measured >= total {
            return;
        }
        // The two fonts the details row uses: the name at 13 px, the folder at
        // its smaller 12 px.
        let name_font = FontId::proportional(13.0);
        let path_font = FontId::proportional(12.0);
        let root = self
            .search
            .as_ref()
            .map(|s| s.root.clone())
            .unwrap_or_default();
        let first = self.search_measured;
        let mut name_px = self.search_name_px;
        let mut path_px = self.search_path_px;
        let mut size_px = self.search_size_px;
        let mut date_px = self.search_date_px;
        let results = self
            .search
            .as_ref()
            .map(|s| &s.results[..])
            .unwrap_or(&[]);
        for entry in &results[first..] {
            let w = ui
                .fonts_mut(|f| f.layout_no_wrap(entry.name.clone(), name_font.clone(), theme::text()))
                .size()
                .x;
            name_px = name_px.max(w);
            if let Some(folder) = hit_folder(&root, &entry.path) {
                let w = ui
                    .fonts_mut(|f| f.layout_no_wrap(folder, path_font.clone(), theme::text()))
                    .size()
                    .x;
                path_px = path_px.max(w);
            }
            // The two numeric cells, so they can hand their slack to the text
            // columns when the row is tight instead of holding a fixed share.
            let w = ui
                .fonts_mut(|f| f.layout_no_wrap(size_label(entry), path_font.clone(), theme::text()))
                .size()
                .x;
            size_px = size_px.max(w);
            let w = ui
                .fonts_mut(|f| {
                    f.layout_no_wrap(fsutil::short_date(entry.modified), path_font.clone(), theme::text())
                })
                .size()
                .x;
            date_px = date_px.max(w);
        }
        self.search_name_px = name_px;
        self.search_path_px = path_px;
        self.search_size_px = size_px;
        self.search_date_px = date_px;
        self.search_measured = total;
    }

    fn ui_details(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        // Before any column is placed: what the two text columns have to hold.
        self.measure_search_columns(ui);
        let width = ui.available_width().max(200.0);
        let header_h = 25.0;
        let (hrect, _) = ui.allocate_exact_size(Vec2::new(width, header_h), Sense::hover());
        ui.painter().rect_filled(hrect, 0.0, theme::header_bg());
        let cols = self.details_columns(hrect);
        let (name_c, size_c, date_c) = (cols.name, cols.size, cols.date);
        // Added feature: the folder column, while a search is showing. It is a
        // label rather than a button: the rows are found by content, not
        // ordered by folder, so a header that pretended to sort would lie.
        if let Some(path_c) = cols.path {
            ui.painter().text(
                Pos2::new(path_c.left() + 2.0, path_c.center().y),
                Align2::LEFT_CENTER,
                "Path",
                FontId::proportional(12.0),
                theme::text_dim(),
            );
        }
        let labels = [
            (SortKey::Name, "Name", name_c, Align2::LEFT_CENTER),
            (SortKey::Size, "Size", size_c, Align2::RIGHT_CENTER),
            (SortKey::Modified, "Modified", date_c, Align2::RIGHT_CENTER),
        ];
        for (key, label, rect, align) in labels {
            let sort_here = self.sort == key;
            let color = if sort_here { theme::text() } else { theme::text_dim() };
            let font = FontId::proportional(12.0);
            let text_w = ui
                .fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), color))
                .size()
                .x;
            let arrow_w = if sort_here { 11.0 } else { 0.0 };
            let anchor = if align == Align2::LEFT_CENTER {
                Pos2::new(rect.left(), rect.center().y)
            } else {
                Pos2::new(rect.right() - arrow_w, rect.center().y)
            };
            ui.painter().text(anchor, align, label, font, color);
            if sort_here {
                let cx = if align == Align2::LEFT_CENTER {
                    rect.left() + text_w + 6.0
                } else {
                    rect.right() - 5.0
                };
                let cy = rect.center().y;
                let tri = if self.asc {
                    vec![
                        Pos2::new(cx - 4.0, cy + 2.5),
                        Pos2::new(cx + 4.0, cy + 2.5),
                        Pos2::new(cx, cy - 3.0),
                    ]
                } else {
                    vec![
                        Pos2::new(cx - 4.0, cy - 2.5),
                        Pos2::new(cx + 4.0, cy - 2.5),
                        Pos2::new(cx, cy + 3.0),
                    ]
                };
                ui.painter()
                    .add(egui::Shape::convex_polygon(tri, theme::text_dim(), egui::epaint::PathStroke::NONE));
            }
            let resp = ui.interact(rect, Id::new(("details_head", self.id, label)), Sense::click());
            if resp.clicked() {
                if self.sort == key {
                    self.asc = !self.asc;
                } else {
                    self.sort = key;
                    self.asc = true;
                }
                self.sort_entries();
            }
        }
        // Vertical column separators, as in the original header.
        let mut seps = vec![size_c.left() - 6.0, date_c.left() - 6.0];
        if let Some(path_c) = cols.path {
            seps.push(path_c.left() - 6.0);
        }
        for x in seps {
            ui.painter().line_segment(
                [Pos2::new(x, hrect.top() + 3.0), Pos2::new(x, hrect.bottom() - 3.0)],
                Stroke::new(1.0, theme::separator()),
            );
        }
        ui.painter().line_segment(
            [Pos2::new(hrect.left(), hrect.bottom()), Pos2::new(hrect.right(), hrect.bottom())],
            Stroke::new(1.0, theme::border()),
        );

        let icon_sz = self.icon_size().clamp(16.0, theme::MAX_ICON_PX);
        let row_h = (icon_sz + 12.0).max(26.0);
        let count = self.display_entries().len();
        if count == 0 {
            self.empty_message(ui, vctx);
            return;
        }
        let scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, count, |ui, range| {
                for idx in range {
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    if self.scroll_to == Some(idx) {
                        ui.scroll_to_rect(rect, Some(Align::Center));
                    }
                    self.draw_details_row(ui, rect, &resp, idx, vctx);
                }
            });
        if self.scroll_to.is_some() {
            self.scroll_to = None;
        }
        let _ = scroll;
    }

    fn draw_details_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &Response,
        idx: usize,
        vctx: &mut ViewCtx,
    ) {
        let Some(entry) = self.display_entries().get(idx).cloned() else {
            return;
        };
        let selected = self.sel.contains(&idx);
        let bg = if selected {
            if vctx.active || !vctx.split {
                theme::row_sel_active()
            } else {
                theme::row_sel_inactive()
            }
        } else if resp.hovered() {
            theme::row_hover()
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 0.0, bg);
        }
        ui.painter().line_segment(
            [Pos2::new(rect.left() + 4.0, rect.bottom()), Pos2::new(rect.right() - 4.0, rect.bottom())],
            Stroke::new(1.0, theme::separator().gamma_multiply(0.55)),
        );

        let cols = self.details_columns(rect);
        let (name_c, size_c, date_c) = (cols.name, cols.size, cols.date);
        // The icon follows the zoom level, as it does in the original's details
        // view: Ctrl+Plus makes the rows grow instead of only changing the label
        // in the status bar.
        let icon_sz = self.icon_size().clamp(16.0, theme::MAX_ICON_PX);
        let icon_rect = Rect::from_center_size(
            Pos2::new(name_c.left() + icon_sz * 0.5 + 2.0, rect.center().y),
            Vec2::splat(icon_sz),
        );
        let mut drew_preview = false;
        if self.previews && entry.has_preview() {
            let px = (icon_sz * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px.max(16)) {
                let _ = draw_preview(ui.painter(), &tex, icon_rect);
                drew_preview = true;
            }
        }
        if !drew_preview {
            theme::draw_kind_icon_tinted(
                ui.painter(),
                icon_rect,
                if entry.is_symlink { Kind::Link } else { entry.kind },
                entry.broken,
                tint_of(&entry),
            );
        }

        let text_color = theme::text();
        let font = FontId::proportional(13.0);
        let name_x = name_c.left() + icon_sz + 8.0;
        let max_w = (name_c.right() - name_x - 8.0).max(20.0);

        let mut draw_label = true;
        let mut commit_rename: Option<String> = None;
        let mut cancel_rename = false;
        if let Some(rename) = self.rename.as_mut() {
            if rename.index == idx {
                draw_label = false;
                let edit_rect = Rect::from_min_size(
                    Pos2::new(name_x, rect.top() + 2.0),
                    Vec2::new(max_w, rect.height() - 4.0),
                );
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(edit_rect)
                        .layout(Layout::top_down(Align::Min)),
                );
                let response = child.add(
                    egui::TextEdit::singleline(&mut rename.text)
                        .font(font.clone())
                        .margin(egui::Margin::symmetric(3, 1)),
                );
                if !rename.focus {
                    response.request_focus();
                    let len = rename.text.chars().count();
                    theme::select_all_text(&child.ctx(), response.id, len);
                    rename.focus = true;
                }
                let enter = child.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = child.input(|i| i.key_pressed(egui::Key::Escape));
                if enter {
                    commit_rename = Some(rename.text.trim().to_string());
                } else if escape {
                    cancel_rename = true;
                }
            }
        }
        if draw_label {
            // `ElidingMode` default `Middle` (`ContentDisplaySettings` ctor
            // @0x2b29b0): the name keeps its start and its extension.
            let label = theme::elide_name(ui, &entry.name, max_w, font.clone());
            ui.painter().text(
                Pos2::new(name_x, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                text_color,
            );
        } else if let Some(new_name) = commit_rename {
            self.rename = None;
            if !new_name.is_empty() && new_name != entry.name {
                vctx.reqs.push(Request::Rename(entry.path.clone(), new_name));
            }
        } else if cancel_rename {
            self.rename = None;
        }

        // Added feature: the folder a search hit was found in, written relative
        // to the folder the search started from — the bar above already names
        // that folder, so repeating the same prefix down every row would spend
        // the column on nothing. A hit sitting directly in the searched folder
        // has no folder of its own to add.
        if let Some(path_c) = cols.path {
            let folder = self
                .search
                .as_ref()
                .and_then(|s| hit_folder(&s.root, &entry.path))
                .unwrap_or_else(|| "—".to_string());
            let font = FontId::proportional(12.0);
            let max_w = (path_c.width() - 6.0).max(20.0);
            let label = theme::elide_name(ui, &folder, max_w, font.clone());
            ui.painter().text(
                Pos2::new(path_c.left() + 2.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font,
                theme::text_dim(),
            );
        }

        let size_text = size_label(&entry);
        ui.painter().text(
            Pos2::new(size_c.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            size_text,
            FontId::proportional(12.0),
            theme::text_dim(),
        );
        ui.painter().text(
            Pos2::new(date_c.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            fsutil::short_date(entry.modified),
            FontId::proportional(12.0),
            theme::text_dim(),
        );

        self.handle_row_interaction(ui, rect, resp, idx, vctx);
    }

    // -----------------------------------------------------------------------
    // Icons view
    // -----------------------------------------------------------------------

    fn ui_icons(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let count = self.display_entries().len();
        if count == 0 {
            self.empty_message(ui, vctx);
            return;
        }
        // Added feature: a folder of pictures gets the picture options in
        // reach without switching to the gallery — size up to 400 px, names,
        // dimensions, rounded corners and previews.
        if self.display_entries().iter().any(|e| e.is_image()) {
            self.picture_options_bar(ui, vctx);
        }
        let icon = self.icon_size().clamp(16.0, theme::MAX_ICON_PX);
        let dims_shown = self.gallery_dims && self.previews;
        // The size line is drawn in the same font and size as the name now, so
        // the cell keeps a whole caption line's room for it — measured from the
        // font itself rather than from the old 15 px, which was sized for the
        // smaller, dimmer line.
        let dims_extra = if dims_shown {
            let line = ui
                .painter()
                .layout_no_wrap(
                    "1920×1080:AR 1.78:MP 2.07".to_owned(),
                    crate::gallery::caption_font(),
                    theme::text(),
                )
                .size()
                .y;
            (line + 3.0).ceil()
        } else {
            0.0
        };
        let cell_h = icon_cell_height(icon) + dims_extra;
        // Cell width comes from `DolphinItemListView::updateGridSize` @0x26d940:
        // every cell at a given zoom is the same width, sized for the label
        // (exp(zoom/13) * 64 * averageCharWidth/9 + 48, at least exp(zoom/13)*4
        // + the icon). The width of the label therefore decides how many
        // pictures share a row, exactly as in the original.
        let zoom = self.zoom_level();
        // `QFontMetrics::averageCharWidth` in the original; 'n' at the label
        // size is the same width to within a pixel for the fonts in use.
        let avg_char = ui
            .painter()
            .layout_no_wrap("n".to_owned(), egui::FontId::proportional(13.0), theme::text())
            .size()
            .x
            .max(1.0);
        let avail = ui.available_width().max(88.0);
        let cell_w = theme::icon_grid_cell_width(zoom, icon, avg_char);
        // ...but that is only the *widest* a cell ever needs to be. A cell is
        // sized for the picture it holds, so a portrait photo does not reserve
        // the empty strip on either side of itself: the picture is fitted into
        // the square icon box by the original (drawPixmap @0x135bb0 keeps the
        // aspect and centres the pixmap), and the cell simply stops at the
        // picture's own edge. See icon_cell_width / label_floor below.
        let previews = self.previews;
        let count = self.display_entries().len();
        let widths: Vec<f32> = self
            .display_entries()
            .iter()
            .map(|entry| {
                let aspect = if previews && entry.has_preview() {
                    vctx.thumbs.aspect(&entry.path)
                } else {
                    1.0
                };
                icon_cell_width(icon, aspect).min(cell_w)
            })
            .collect();
        let mut rows = pack_icon_rows(&widths, avail, ICON_CELL_GAP);
        // When the list is long enough to scroll, the scroll bar takes its strip
        // out of the inside width; pack again with that strip removed so the last
        // cell of a row is never pushed under the bar.
        let view_h = ui.available_height();
        if rows.len() as f32 * (cell_h + 4.0) > view_h {
            rows = pack_icon_rows(&widths, (avail - 14.0).max(120.0), ICON_CELL_GAP);
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, cell_h, rows.len(), |ui, range| {
                for row in range {
                    let (start, end) = rows[row];
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = ICON_CELL_GAP;
                        for idx in start..end {
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(widths[idx], cell_h),
                                Sense::click(),
                            );
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            if self.scroll_to == Some(idx) {
                                ui.scroll_to_rect(rect, Some(Align::Center));
                            }
                            self.draw_icon_cell(ui, rect, &resp, idx, icon, vctx);
                        }
                    });
                }
            });
        if self.scroll_to.is_some() {
            self.scroll_to = None;
        }
    }

    /// The picture options row: the same switches the gallery bar carries, so
    /// they are in reach while browsing a folder of pictures in the icon view.
    /// Size walks the original's zoom ladder, now continued to 400 px.
    fn picture_options_bar(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let vi = self.view.index();
        let mut zoom = self.zoom[vi] as f32;
        let mut zoom_changed = false;
        let mut changed = false;
        let mut names = self.gallery_names;
        let mut dims = self.gallery_dims;
        let mut round = self.gallery_round;
        let mut previews = self.previews;

        egui::Frame::new()
            .fill(theme::header_bg())
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                let row = Vec2::new(ui.available_width(), 22.0);
                ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(
                        egui::RichText::new("Pictures:")
                            .size(11.5)
                            .color(theme::text_dim()),
                    );
                    let slider = theme::strip_slider(
                        ui,
                        &mut zoom,
                        0.0..=theme::MAX_ZOOM as f32,
                        150.0,
                    );
                    if slider.changed() {
                        zoom_changed = true;
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} px",
                            theme::icon_size_for_zoom(zoom.round() as u8) as u32
                        ))
                        .size(11.0)
                        .color(theme::text_faint()),
                    );
                    if ui
                        .add(egui::Checkbox::new(
                            &mut names,
                            egui::RichText::new("Names").size(11.5),
                        ))
                        .on_hover_text("Show the file name under each thumbnail")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .add(egui::Checkbox::new(
                            &mut dims,
                            egui::RichText::new("Dimensions").size(11.5),
                        ))
                        .on_hover_text("Show width × height, aspect ratio and megapixels under each picture")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .add(egui::Checkbox::new(
                            &mut round,
                            egui::RichText::new("Rounded").size(11.5),
                        ))
                        .on_hover_text("Round the corners of the picture thumbnails")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .add(egui::Checkbox::new(
                            &mut previews,
                            egui::RichText::new("Previews").size(11.5),
                        ))
                        .on_hover_text("Draw the pictures themselves instead of a file-type icon")
                        .changed()
                    {
                        changed = true;
                    }
                });
            });

        if zoom_changed {
            self.zoom[vi] = (zoom.round() as i32).clamp(0, theme::MAX_ZOOM as i32) as u8;
            vctx.reqs.push(Request::ZoomChanged);
        }
        if changed {
            self.gallery_names = names;
            self.gallery_dims = dims;
            self.gallery_round = round;
            self.previews = previews;
            vctx.reqs.push(Request::GalleryChanged);
            vctx.reqs.push(Request::ZoomChanged);
        }
    }

    fn draw_icon_cell(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &Response,
        idx: usize,
        icon: f32,
        vctx: &mut ViewCtx,
    ) {
        let Some(entry) = self.display_entries().get(idx).cloned() else {
            return;
        };
        let selected = self.sel.contains(&idx);
        let bg = if selected {
            if vctx.active || !vctx.split {
                theme::row_sel_active()
            } else {
                theme::row_sel_inactive()
            }
        } else if resp.hovered() {
            theme::row_hover()
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink(4.0), CornerRadius::same(4), bg);
        }
        let aspect = if self.previews && entry.has_preview() {
            vctx.thumbs.aspect(&entry.path)
        } else {
            1.0
        };
        // The picture keeps its own shape inside the square icon box: a tall
        // photo is tall and narrow, a wide one short and wide, both centred in
        // the box so a row of mixed shapes looks even.
        let shape = thumb_size(icon, aspect);
        let icon_rect = Rect::from_min_size(
            Pos2::new(
                rect.center().x - shape.x / 2.0,
                rect.top() + 8.0 + (icon - shape.y) / 2.0,
            ),
            shape,
        );
        let mut drew_thumb = false;
        if self.previews && entry.has_preview() {
            // Previews are shown from the smallest zoom level up, as in the
            // original, and the thumbnail is rendered at the icon size times the
            // screen scale so zooming in always gives a sharper, bigger picture.
            let px = (icon * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px) {
                if let Some(drawn) = draw_preview(ui.painter(), &tex, icon_rect) {
                    // Added feature: the rounded-corner switch, on by default.
                    // The mask takes the colour actually behind the picture so a
                    // selected or hovered cell keeps its own tint in the corners.
                    if self.gallery_round {
                        let mask = if bg == Color32::TRANSPARENT { theme::view_bg() } else { bg };
                        crate::gallery::round_picture(
                            ui.painter(),
                            drawn,
                            picture_radius(icon),
                            mask,
                        );
                    }
                    drew_thumb = true;
                }
            }
        }
        if !drew_thumb {
            theme::draw_kind_icon_tinted(
                ui.painter(),
                Rect::from_center_size(icon_rect.center(), Vec2::splat(icon)),
                if entry.is_symlink { Kind::Link } else { entry.kind },
                entry.broken,
                tint_of(&entry),
            );
        }

        // Labels sit under the icon box's bottom edge, not under the picture, so
        // they stay on one line across the whole row whatever the shapes are.
        let label_y = icon_rect.top() + icon + 6.0;
        let max_w = (rect.width() - 6.0).max(24.0);
        // One caption font for both lines: the name and, when switched on, the
        // size line under it.
        let font = crate::gallery::caption_font();
        let mut caption_y = label_y;
        // Added feature: the Names switch hides the label under each picture.
        if self.gallery_names {
            let mut lines = wrap_label(
                ui,
                &entry.name,
                max_w,
                font.clone(),
                if icon >= 32.0 { 2 } else { 1 },
            );
            // A second line holding one or two leftover letters ("…jpg" cut to
            // "g") looks like a mistake; keep the single fuller line instead.
            if lines.len() == 2 && lines[1].chars().count() < 3 {
                lines.truncate(1);
            }
            for (i, line) in lines.iter().enumerate() {
                ui.painter().text(
                    Pos2::new(rect.center().x, label_y + 11.0 + i as f32 * 13.0),
                    Align2::CENTER_CENTER,
                    line,
                    font.clone(),
                    if selected { theme::on_band() } else { theme::text() },
                );
            }
            caption_y = label_y + 11.0 + (lines.len().saturating_sub(1)) as f32 * 13.0;
        }
        // Added feature: the Dimensions switch prints the picture's own size,
        // aspect ratio and megapixels under its name — in the name's own font,
        // size and colour, so the two lines read as one caption.
        if self.gallery_dims && self.previews && !entry.is_dir && entry.is_image() {
            let size = crate::search::probe_image_size(&entry.path);
            let caption = crate::gallery::dims_caption(size);
            if !caption.is_empty() {
                // One caption step below the last name line, with a little air
                // between the two so a descender never touches the digits.
                let dy = if self.gallery_names { 14.0 } else { 11.0 };
                ui.painter().text(
                    Pos2::new(rect.center().x, caption_y + dy),
                    Align2::CENTER_CENTER,
                    theme::elide(ui, &caption, max_w, font.clone()),
                    font.clone(),
                    crate::gallery::caption_colour(selected),
                );
            }
        }

        self.handle_row_interaction(ui, rect, resp, idx, vctx);
    }

    // -----------------------------------------------------------------------
    // Compact view
    // -----------------------------------------------------------------------

    fn ui_compact(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let count = self.display_entries().len();
        if count == 0 {
            self.empty_message(ui, vctx);
            return;
        }
        let icon = self.icon_size().clamp(16.0, theme::MAX_ICON_PX);
        let row_h = (icon + 6.0).max(22.0);
        // Compact cell width from `DolphinItemListView::updateGridSize`
        // @0x26d940: iconSize + 8 + 5 * fontHeight.
        let font_h = ui.text_style_height(&egui::TextStyle::Body);
        let cell_w = theme::compact_cell_width(icon, font_h);
        let avail = ui.available_width();
        let cols = ((avail - 8.0) / cell_w).floor().max(1.0) as usize;
        let rows = (count + cols - 1) / cols;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for col in 0..cols {
                            let idx = row * cols + col;
                            if idx >= count {
                                break;
                            }
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(cell_w, row_h), Sense::click());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            if self.scroll_to == Some(idx) {
                                ui.scroll_to_rect(rect, Some(Align::Center));
                            }
                            self.draw_compact_cell(ui, rect, &resp, idx, vctx);
                        }
                    });
                }
            });
        if self.scroll_to.is_some() {
            self.scroll_to = None;
        }
    }

    fn draw_compact_cell(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &Response,
        idx: usize,
        vctx: &mut ViewCtx,
    ) {
        let Some(entry) = self.display_entries().get(idx).cloned() else {
            return;
        };
        let selected = self.sel.contains(&idx);
        let bg = if selected {
            if vctx.active || !vctx.split {
                theme::row_sel_active()
            } else {
                theme::row_sel_inactive()
            }
        } else if resp.hovered() {
            theme::row_hover()
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink2(Vec2::new(3.0, 1.0)), CornerRadius::same(3), bg);
        }
        let icon = self.icon_size().clamp(16.0, theme::MAX_ICON_PX);
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 9.0 + icon / 2.0, rect.center().y),
            Vec2::splat(icon),
        );
        let mut drew_preview = false;
        if self.previews && entry.has_preview() {
            let px = (icon * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px.max(16)) {
                if let Some(drawn) = draw_preview(ui.painter(), &tex, icon_rect) {
                    if self.gallery_round {
                        let mask = if bg == Color32::TRANSPARENT { theme::view_bg() } else { bg };
                        crate::gallery::round_picture(
                            ui.painter(),
                            drawn,
                            picture_radius(icon),
                            mask,
                        );
                    }
                    drew_preview = true;
                }
            }
        }
        if !drew_preview {
            theme::draw_kind_icon_tinted(
                ui.painter(),
                icon_rect,
                if entry.is_symlink { Kind::Link } else { entry.kind },
                entry.broken,
                tint_of(&entry),
            );
        }
        let font = FontId::proportional(12.5);
        let label_x = rect.left() + icon + 18.0;
        let max_w = (rect.right() - label_x - 8.0).max(20.0);
        let label = theme::elide_name(ui, &entry.name, max_w, font.clone());
        ui.painter().text(
            Pos2::new(label_x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            if selected { theme::on_band() } else { theme::text() },
        );
        self.handle_row_interaction(ui, rect, resp, idx, vctx);
    }

    // -----------------------------------------------------------------------
    // Shared row behaviour
    // -----------------------------------------------------------------------

    pub(crate) fn handle_row_interaction(
        &mut self,
        ui: &mut egui::Ui,
        _rect: Rect,
        resp: &Response,
        idx: usize,
        vctx: &mut ViewCtx,
    ) {
        if resp.clicked() {
            let mods = ui.input(|i| i.modifiers);
            if mods.shift {
                self.select_range(idx);
            } else if mods.ctrl || mods.command {
                self.toggle_select(idx);
            } else {
                self.select_single(idx);
            }
        }
        if resp.double_clicked() {
            self.select_single(idx);
            let picture = self
                .display_entries()
                .get(idx)
                .map(|e| !e.is_dir && e.is_image())
                .unwrap_or(false);
            if self.pictures && picture {
                // In the gallery a double click opens the picture in the
                // viewer, the way clicking one opens it in the browser.
                if let Some(entry) = self.display_entries().get(idx) {
                    vctx.reqs.push(Request::OpenViewer(entry.path.clone(), false));
                }
            } else {
                self.open_entry(idx, vctx.reqs);
            }
        }
        if resp.middle_clicked() {
            if let Some(entry) = self.display_entries().get(idx).cloned() {
                vctx.reqs.push(Request::OpenInNewTab(entry.path));
            }
        }
        // Added feature, asked for: a file carried out of the window and into
        // another program. The press that marked the item becomes the drag once
        // the pointer has travelled far enough — so a click is still a click.
        // The press has to be remembered by *where it began*, not by the row's
        // own "held" flag: a row with a click sense lets that flag go as soon as
        // the pointer leaves it, which is exactly what a drag does.
        let carried = ui.input(|i| {
            match (i.pointer.press_origin(), i.pointer.latest_pos()) {
                (Some(from), Some(to)) if _rect.contains(from) && i.pointer.any_down() => {
                    crate::dragout::far_enough(from, to)
                }
                _ => false,
            }
        });
        if std::env::var_os("DOLPHINCRAFT_DRAG_TRACE").is_some() && carried {
            eprintln!("drag trace: row {idx} carried away, handing the files over");
        }
        {
            if carried {
                let files: Vec<std::path::PathBuf> = if self.sel.contains(&idx) && self.sel.len() > 1 {
                    self.selected_paths()
                } else {
                    self.display_entries()
                        .get(idx)
                        .map(|entry| vec![entry.path.clone()])
                        .unwrap_or_default()
                };
                if !files.is_empty() {
                    vctx.reqs.push(Request::DragOut(files));
                }
            }
        }
        let current_dir = self.current_dir();
        let entry = self.display_entries().get(idx).cloned();
        if let Some(entry) = entry {
            let selected_paths: Vec<std::path::PathBuf> = if self.sel.contains(&idx) && self.sel.len() > 1 {
                self.selected_paths()
            } else {
                vec![entry.path.clone()]
            };
            let in_trash = self.is_trash();
            let trash_empty = self.display_entries().is_empty();
            let clipboard_has = !vctx.clipboard.paths.is_empty();
            resp.context_menu(|ui| {
                ui.set_min_width(210.0);
                if in_trash {
                    if ui.button("Restore to Former Location").clicked() {
                        vctx.reqs.push(Request::Restore(selected_paths.clone()));
                        ui.close();
                    }
                    if ui.button(crate::DELETE_PERMANENT_LABEL).clicked() {
                        vctx.reqs.push(Request::DeletePermanent(vec![entry.path.clone()]));
                        ui.close();
                    }
                    // The Trash's own row, greyed out while the Trash is empty,
                    // exactly as the original's context menu has it.
                    let mut empty = false;
                    ui.add_enabled_ui(!trash_empty, |ui| {
                        if ui.button(crate::EMPTY_TRASH_LABEL).clicked() {
                            empty = true;
                        }
                    });
                    if empty {
                        vctx.reqs.push(Request::EmptyTrash);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Properties").clicked() {
                        vctx.reqs.push(Request::Properties(selected_paths.clone()));
                        ui.close();
                    }
                    return;
                }
                if ui.button("Open").clicked() {
                    vctx.reqs.push(Request::OpenPath(entry.path.clone()));
                    ui.close();
                }
                // The original's "Open With": the preferred program's name in
                // a row of its own ("Open with Kate"), then a submenu holding
                // the other programs that say they open this type, closed by
                // "Other Application…" (libKF6KIOWidgets.so.6, 0x11c24a /
                // 0x11c356 / 0x11c387).
                {
                    let mime = crate::apps::mime_of(&entry.path);
                    let preferred = crate::apps::default_for(&mime)
                        .and_then(|id| crate::apps::find(&id));
                    if let Some(app) = preferred.clone() {
                        if !entry.is_dir
                            && ui
                                .button(format!("Open with {}", app.name))
                                .clicked()
                        {
                            vctx.reqs.push(Request::OpenWith {
                                id: app.id.clone(),
                                paths: selected_paths.clone(),
                                remember: None,
                            });
                            ui.close();
                        }
                    }
                    let openers: Vec<crate::apps::AppEntry> = crate::apps::openers_for(&mime)
                        .into_iter()
                        .filter(|a| preferred.as_ref().map(|p| p.id != a.id).unwrap_or(true))
                        .collect();
                    let title = if entry.is_dir {
                        "Open Folder With"
                    } else {
                        "Open With"
                    };
                    ui.menu_button(title, |ui| {
                        ui.set_min_width(220.0);
                        let mut other = false;
                        for app in openers.iter().take(12) {
                            if ui.button(&app.name).clicked() {
                                vctx.reqs.push(Request::OpenWith {
                                    id: app.id.clone(),
                                    paths: selected_paths.clone(),
                                    remember: None,
                                });
                                ui.close();
                            }
                        }
                        if openers.is_empty() {
                            ui.label(
                                egui::RichText::new("No program claims this kind of file.")
                                    .size(11.5)
                                    .color(theme::text_faint()),
                            );
                        }
                        ui.separator();
                        if ui.button("Other Application…").clicked() {
                            other = true;
                        }
                        if other {
                            vctx.reqs
                                .push(Request::OpenWithChoose(selected_paths.clone()));
                            ui.close();
                        }
                    });
                }
                // Added feature: the archive rows, in the place Ark's plugin puts
                // its own in the original. Nothing is marked as "compressed" by
                // the file view, so the row simply opens the window.
                if ui.button("Compress…").clicked() {
                    let dir = self.path.clone();
                    vctx.reqs.push(Request::Compress(selected_paths.clone(), dir));
                    ui.close();
                }
                if !entry.is_dir && crate::archive::is_archive(&entry.path) && ui.button("Extract…").clicked() {
                    vctx.reqs.push(Request::Extract(entry.path.clone()));
                    ui.close();
                }
                if !entry.is_dir && entry.is_image() && ui.button("Open in Picture Viewer").clicked() {
                    vctx.reqs.push(Request::OpenViewer(entry.path.clone(), false));
                    ui.close();
                }
                if !entry.is_dir && entry.is_image() && ui.button("Picture Slideshow").clicked() {
                    vctx.reqs.push(Request::OpenViewer(entry.path.clone(), true));
                    ui.close();
                }
                // Added this round, as asked: *Resize Image…* — the one number
                // the window asks for is a size in megapixels, and the picture's
                // shape is kept while it is scaled to it. It acts on every
                // picture in the selection, so a batch of photographs goes in
                // one go.
                {
                    let pictures: Vec<std::path::PathBuf> = selected_paths
                        .iter()
                        .filter(|p| !p.is_dir() && crate::imageresize::supported(p))
                        .cloned()
                        .collect();
                    if !pictures.is_empty() {
                        let label = if pictures.len() > 1 {
                            "Resize Images…"
                        } else {
                            "Resize Image…"
                        };
                        if ui
                            .button(label)
                            .on_hover_text("Scale to a size in megapixels, keeping the picture's shape")
                            .clicked()
                        {
                            vctx.reqs.push(Request::Resize(pictures));
                            ui.close();
                        }
                    }
                }
                if entry.is_dir {
                    if ui.button("Open in New Tab").clicked() {
                        vctx.reqs.push(Request::OpenInNewTab(entry.path.clone()));
                        ui.close();
                    }
                    if ui.button("Open in New Window").clicked() {
                        crate::spawn_folder_window(&entry.path);
                        ui.close();
                    }
                    if vctx.split && ui.button("Open in Other View").clicked() {
                        vctx.reqs.push(Request::NavigateOther(entry.path.clone()));
                        ui.close();
                    }
                }
                if entry.is_dir {
                    ui.separator();
                    let targets: Vec<std::path::PathBuf> = {
                        let dirs: Vec<std::path::PathBuf> = selected_paths
                            .iter()
                            .filter(|p| p.is_dir())
                            .cloned()
                            .collect();
                        if dirs.is_empty() {
                            vec![entry.path.clone()]
                        } else {
                            dirs
                        }
                    };
                    let current = entry.tint;
                    ui.menu_button("Folder Colour", |ui| {
                        ui.set_min_width(190.0);
                        match theme::colour_menu(ui, current) {
                            Some(theme::ColourPick::Set(rgb)) => {
                                vctx.reqs
                                    .push(Request::FolderColor(targets.clone(), Some(rgb)));
                                ui.close();
                            }
                            Some(theme::ColourPick::Clear) => {
                                vctx.reqs.push(Request::FolderColor(targets.clone(), None));
                                ui.close();
                            }
                            None => {}
                        }
                    });
                }
                ui.separator();
                if ui.button("Cut").clicked() {
                    vctx.reqs.push(Request::Cut);
                    ui.close();
                }
                if ui.button("Copy").clicked() {
                    vctx.reqs.push(Request::Copy);
                    ui.close();
                }
                if clipboard_has && ui.button("Paste").clicked() {
                    vctx.reqs.push(Request::Paste);
                    ui.close();
                }
                if ui.button("Rename…").clicked() {
                    vctx.reqs.push(Request::BeginRename(idx));
                    ui.close();
                }
                if ui.button("Duplicate Here").clicked() {
                    vctx.reqs.push(Request::Duplicate(selected_paths.clone()));
                    ui.close();
                }
                // Added feature: the original's two Other View rows (enabled
                // only with two views open, and only when its own four tests
                // pass) plus the Copy To… / Move To… pair the original installs
                // from KIO's context menu.
                ui.separator();
                for mode in [crate::copymove::Mode::Copy, crate::copymove::Mode::Move] {
                    let verdict = crate::copymove::verdict(
                        mode,
                        &crate::copymove::Facts {
                            split: vctx.split,
                            selected: selected_paths.len(),
                            already_there: vctx
                                .other_dir
                                .as_ref()
                                .map(|d| crate::copymove::already_there(d, &selected_paths))
                                .unwrap_or(false),
                            dest_writable: vctx
                                .other_dir
                                .as_ref()
                                .map(|d| crate::copymove::writable(d))
                                .unwrap_or(false),
                            may_move: crate::copymove::may_move(&selected_paths),
                        },
                    );
                    let hint = verdict
                        .why
                        .clone()
                        .unwrap_or_else(|| mode.whats_this().to_string());
                    let mut clicked = false;
                    ui.add_enabled_ui(verdict.ok, |ui| {
                        clicked = ui
                            .button(format!("{}  {}", mode.other_label(), mode.other_shortcut()))
                            .on_hover_text(hint)
                            .clicked();
                    });
                    if clicked {
                        vctx.reqs
                            .push(Request::CopyToOther(mode == crate::copymove::Mode::Move));
                        ui.close();
                    }
                }
                for mode in [crate::copymove::Mode::Copy, crate::copymove::Mode::Move] {
                    if ui
                        .button(mode.chooser_label())
                        .on_hover_text("Into a folder you choose; nothing already there is replaced")
                        .clicked()
                    {
                        vctx.reqs.push(Request::OpenCopyChooser(mode));
                        ui.close();
                    }
                }
                if ui.button("Copy Location").clicked() {
                    ui.ctx().copy_text(entry.path.display().to_string());
                    ui.close();
                }
                // Added feature: the prompt a picture was made with, straight
                // from its own file — for every picture in the selection when
                // several are picked, so a whole batch goes at once.
                let pictures: Vec<std::path::PathBuf> = selected_paths
                    .iter()
                    .filter(|p| crate::imagemeta::is_picture_path(p))
                    .cloned()
                    .collect();
                if !pictures.is_empty() {
                    let label = if pictures.len() > 1 {
                        "Copy Prompts"
                    } else {
                        "Copy Prompt"
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Copy the generation prompt stored in the picture")
                        .clicked()
                    {
                        let (text, n) = crate::imagemeta::prompts_of(&pictures);
                        if n == 0 {
                            vctx.reqs.push(Request::Toast(
                                "No generation prompt is stored in these pictures.".into(),
                            ));
                        } else {
                            ui.ctx().copy_text(text);
                            vctx.reqs.push(Request::Toast(if n == 1 {
                                "Prompt copied to the clipboard.".into()
                            } else {
                                format!("{n} prompts copied to the clipboard.")
                            }));
                        }
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Move to Trash").clicked() {
                    vctx.reqs.push(Request::Trash(selected_paths.clone()));
                    ui.close();
                }
                if ui.button(crate::DELETE_PERMANENT_LABEL).clicked() {
                    vctx.reqs.push(Request::DeletePermanent(selected_paths.clone()));
                    ui.close();
                }
                ui.separator();
                if entry.is_dir && ui.button("Add Shortcut…").clicked() {
                    vctx.reqs.push(Request::AddBookmark(
                        entry.name.clone(),
                        entry.path.clone(),
                    ));
                    ui.close();
                }
                if ui.button("Open Terminal Here").clicked() {
                    vctx.reqs.push(Request::OpenTerminal(current_dir.clone()));
                    ui.close();
                }
                if ui.button("Properties").clicked() {
                    vctx.reqs.push(Request::Properties(selected_paths.clone()));
                    ui.close();
                }
                if ui.button("Bulk Rename…").clicked() {
                    vctx.reqs.push(Request::BulkRename(selected_paths.clone()));
                    ui.close();
                }
                if entry.is_dir && ui.button("Calculate Folder Size").clicked() {
                    vctx.reqs.push(Request::FolderSize(vec![entry.path.clone()]));
                    ui.close();
                }
            });
        }
    }

    fn empty_message(&self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let _ = vctx;
        let (text, hint) = if let Some(err) = &self.error {
            (err.clone(), String::new())
        } else if self.search.is_some() {
            let s = self.search.as_ref().unwrap();
            if !s.done {
                ("Searching…".to_string(), format!("in {}", s.root.display()))
            } else {
                ("No items matching the search".to_string(), String::new())
            }
        } else if self.is_trash() {
            ("Trash is empty".to_string(), String::new())
        } else if self.filter_on && !self.filter.trim().is_empty() {
            ("No items matching the filter".to_string(), String::new())
        } else if !self.show_hidden
            && self
                .path
                .read_dir()
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
        {
            ("Folder is empty".to_string(), "Hidden files are not shown".to_string())
        } else {
            ("The location is empty.".to_string(), String::new())
        };
        let rect = ui.available_rect_before_wrap();
        let center = Pos2::new(rect.center().x, rect.top() + 90.0);
        let galley = ui.painter().layout(
            text,
            FontId::proportional(14.0),
            theme::text_dim(),
            (rect.width() - 48.0).max(120.0),
        );
        let pos = Pos2::new(center.x - galley.size().x * 0.5, center.y - galley.size().y * 0.5);
        ui.painter().galley(pos, galley, theme::text_dim());
        if !hint.is_empty() {
            ui.painter().text(
                Pos2::new(center.x, center.y + 22.0),
                Align2::CENTER_CENTER,
                hint,
                FontId::proportional(12.0),
                theme::text_faint(),
            );
        }
    }
}

/// Padding between a cell's edge and its picture or label.
pub const ICON_CELL_PAD: f32 = 4.0;
/// Gap between two neighbouring cells.
pub const ICON_CELL_GAP: f32 = 2.0;

/// The size a preview takes inside an `icon` × `icon` box when the picture's
/// width/height ratio is `aspect`.
///
/// This is what the original does with `KPixmapModifier::scale` @0x15e820,
/// called from `KStandardItemListWidget::drawPixmap` @0x135bb0: the aspect is
/// kept and the pixmap is centred in the square box. A picture that is taller
/// than it is wide therefore only fills `aspect * icon` of the box's width.
pub fn thumb_size(icon: f32, aspect: f32) -> Vec2 {
    let icon = icon.clamp(16.0, theme::MAX_ICON_PX);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    if aspect <= 1.0 {
        Vec2::new(icon * aspect, icon)
    } else {
        Vec2::new(icon, icon / aspect)
    }
}

/// The narrowest a label may be squeezed at this icon size: about three
/// quarters of the icon, kept between 40 and 96 px, so a name still shows a
/// couple of words' worth on two lines. Kept as a safety floor for the label;
/// the cell width itself comes from the original's formula in `theme.rs`.
pub fn label_floor(icon: f32) -> f32 {
    (icon * 0.75).clamp(40.0, 96.0)
}

/// How wide one cell is: as wide as the picture inside it plus a small pad on
/// each side, but never narrower than a readable label. This is what stops a
/// portrait photo from reserving the empty strip left and right of itself, and
/// it is always capped by the original's label-based cell width in `theme`.
pub fn icon_cell_width(icon: f32, aspect: f32) -> f32 {
    thumb_size(icon, aspect).x.max(label_floor(icon)) + ICON_CELL_PAD * 2.0
}

/// Height of one row in the icons view: the icon box plus the label underneath.
pub fn icon_cell_height(icon: f32) -> f32 {
    let icon = icon.clamp(16.0, theme::MAX_ICON_PX);
    let label_lines = if icon >= 32.0 { 2.0 } else { 1.0 };
    icon + if label_lines == 2.0 { 46.0 } else { 32.0 }
}

/// Shelf packing: split items into rows, left to right, starting a new row as
/// soon as the next cell no longer fits `avail`. Returns (start, end) index
/// pairs, end exclusive. Unlike a fixed column count this lets a row hold more
/// of the narrow, portrait cells than of the square ones.
pub fn pack_icon_rows(widths: &[f32], avail: f32, gap: f32) -> Vec<(usize, usize)> {
    let avail = avail.max(40.0);
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut used = 0.0f32;
    for (i, w) in widths.iter().enumerate() {
        let w = (*w).max(1.0);
        let needed = if i == start { w } else { used + gap + w };
        if i > start && needed > avail {
            rows.push((start, i));
            start = i;
            used = w;
        } else {
            used = needed;
        }
    }
    if start < widths.len() {
        rows.push((start, widths.len()));
    }
    rows
}

fn measure_text(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), theme::text()))
        .size()
        .x
}

/// Split one over-long run of characters into the part that fits the column and
/// the remainder, so a name without spaces still uses both label lines.
fn split_at_width(ui: &egui::Ui, text: &str, max_w: f32, font: &FontId) -> (String, String) {
    let mut head = String::new();
    let mut tail = String::new();
    let mut full = false;
    for ch in text.chars() {
        if !full {
            let mut probe = head.clone();
            probe.push(ch);
            if measure_text(ui, &probe, font) <= max_w {
                head = probe;
            } else {
                full = true;
                tail.push(ch);
            }
        } else {
            tail.push(ch);
        }
    }
    (head, tail)
}

fn wrap_label(
    ui: &egui::Ui,
    text: &str,
    max_w: f32,
    font: FontId,
    max_lines: usize,
) -> Vec<String> {
    if measure_text(ui, text, &font) <= max_w || max_lines <= 1 {
        return vec![theme::elide(ui, text, max_w, font)];
    }
    // Break opportunities sit after a space, hyphen, underscore or dot, so a
    // tight column can still show "archive-" / "backup.zip" instead of cutting
    // the whole name off after six letters.
    let mut chunks: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if ch == ' ' || ch == '-' || ch == '_' || ch == '.' || ch == '/' {
            chunks.push_back(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push_back(cur);
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    while let Some(chunk) = chunks.pop_front() {
        let fits = {
            let mut probe = current.clone();
            probe.push_str(&chunk);
            measure_text(ui, probe.trim_end(), &font) <= max_w
        };
        if fits {
            current.push_str(&chunk);
            continue;
        }
        if !current.trim().is_empty() {
            lines.push(current.trim_end().to_string());
            current.clear();
            if lines.len() + 1 >= max_lines {
                let mut rest = chunk;
                for c in chunks.drain(..) {
                    rest.push_str(&c);
                }
                lines.push(theme::elide(ui, rest.trim_end(), max_w, font));
                return lines;
            }
            if measure_text(ui, chunk.trim_end(), &font) <= max_w {
                current = chunk;
                continue;
            }
        }
        let (head, tail) = split_at_width(ui, chunk.trim_end(), max_w, &font);
        current = head;
        if !tail.is_empty() {
            chunks.push_front(tail);
        }
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }
    lines
}

/// Width of one scope chip in the find bar.
pub const FIND_CHIP_W: f32 = 78.0;

/// One of the find bar's two scope chips ("This folder" / "In subfolders").
/// Drawn rather than pulled from the style so the pair reads as a two-way
/// switch: the chosen side is filled and accented, the other is an outline.
fn find_chip(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(FIND_CHIP_W, 22.0), Sense::click());
    let (fill, stroke, tint) = if on {
        (
            theme::c(56, 63, 70),
            Stroke::new(1.0, theme::accent()),
            theme::accent(),
        )
    } else if resp.hovered() {
        (theme::row_hover(), Stroke::new(1.0, theme::border()), theme::text())
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, theme::border()), theme::text_dim())
    };
    ui.painter()
        .rect(rect, CornerRadius::same(3), fill, stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.5),
        tint,
    );
    resp
}
