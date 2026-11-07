//! Drawing a folder view: breadcrumb, Details/Icons/Compact layouts,
//! selection, inline rename and context menus.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, Response, Sense,
    Stroke, TextStyle, UiBuilder, Vec2,
};

use crate::fsutil::{self, Kind};
use crate::search::ThumbCache;
use crate::state::{Clipboard, Pane, Request, SortKey, ViewMode};
use crate::theme::{self, *};

/// Room kept for the "…" that stands in for the elided start of a path.
const LEADER_W: f32 = 12.0;

pub struct ViewCtx<'a> {
    pub thumbs: &'a mut ThumbCache,
    pub clipboard: &'a Clipboard,
    pub reqs: &'a mut Vec<Request>,
    pub split: bool,
    pub active: bool,
    pub show_location_bar: bool,
}

/// The colour a folder was given, ready for drawing. Files never take one.
fn tint_of(entry: &fsutil::Entry) -> Option<Color32> {
    if !entry.is_dir {
        return None;
    }
    entry.tint.map(|c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// Draw a preview texture inside `rect` the way the original fits a preview
/// pixmap into an item's icon area: the aspect ratio is kept, the picture is
/// centred and it fills as much of the square as it can, whether that scales it
/// up or down (`KPixmapModifier::scale` @0x15e820, called from
/// `KStandardItemListWidget::drawPixmap` @0x135bb0).
fn draw_preview(painter: &egui::Painter, tex: &egui::TextureHandle, rect: Rect) {
    let src = tex.size();
    if src[0] == 0 || src[1] == 0 || rect.width() <= 0.0 {
        return;
    }
    let scale = (rect.width() / src[0] as f32).min(rect.height() / src[1] as f32);
    let size = Vec2::new(src[0] as f32 * scale, src[1] as f32 * scale);
    painter.image(
        tex.id(),
        Rect::from_center_size(rect.center(), size),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
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
                .fill(HEADER_BG)
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(&mut child, |ui| {
                    egui::Frame::new()
                        .fill(HEADER_BG)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(7, 2))
                        .show(ui, |ui| {
                            ui.set_min_width((ui.available_width() - 4.0).max(60.0));
                            ui.set_min_height(22.0);
                            self.breadcrumb_ui(ui, vctx.reqs);
                        });
                });
        }
        self.filter_bar(&mut child, vctx);

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
                Stroke::new(2.0, ACCENT),
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
        let before = self.zoom[self.view.index()];
        self.zoom_by(steps);
        self.zoom[self.view.index()] != before
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
                    ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), TEXT).size().x)
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
                        .color(TEXT_FAINT)
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
                    ui.label(egui::RichText::new("›").color(TEXT_FAINT).size(12.0));
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
                        ROW_HOVER,
                    );
                }
            }
        });
        if bar.clicked() && !crumb_clicked {
            self.begin_location_edit();
        }
    }

    // -----------------------------------------------------------------------
    // Filter / search bar
    // -----------------------------------------------------------------------

    fn filter_bar(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        if !self.filter_on {
            return;
        }
        let mut stop = false;
        let mut close = false;
        let mut submit = false;
        egui::Frame::new()
            .fill(HEADER_BG)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    theme::draw_glyph(ui.painter(), icon_rect, "search", TEXT_DIM);
                    let searching = self.search.is_some();
                    let extra = if searching { 190.0 } else { 250.0 };
                    let w = (ui.available_width() - extra).max(100.0);
                    let hint = if searching {
                        "Searching subfolders…"
                    } else {
                        "Filter, or press Enter to search in subfolders"
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
                        if self.search.is_some() {
                            self.start_search();
                        }
                    }
                    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                    if searching {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} found",
                                self.search.as_ref().map(|s| s.results.len()).unwrap_or(0)
                            ))
                            .size(11.0)
                            .color(TEXT_FAINT),
                        );
                        if ui.small_button("Stop").clicked() {
                            stop = true;
                        }
                    } else if ui
                        .add(egui::Button::new(
                            egui::RichText::new("Search in subfolders").size(11.0),
                        ))
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("✕").size(11.0))
                                .fill(Color32::TRANSPARENT)
                                .min_size(Vec2::new(22.0, 20.0)),
                        )
                        .on_hover_text("Hide filter bar (Esc)")
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if submit {
            self.start_search();
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
    }

    // -----------------------------------------------------------------------
    // Details view
    // -----------------------------------------------------------------------

    fn column_rects(&self, rect: Rect) -> (Rect, Rect, Rect) {
        let w = rect.width();
        let date_w = (w * 0.28).clamp(64.0, 150.0);
        let size_w = (w * 0.16).clamp(44.0, 88.0);
        let name_w = (w - size_w - date_w).max(w * 0.45);
        let name = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + name_w, rect.bottom()),
        );
        let size = Rect::from_min_max(
            Pos2::new(rect.left() + name_w, rect.top()),
            Pos2::new(rect.left() + name_w + size_w, rect.bottom()),
        );
        let date = Rect::from_min_max(
            Pos2::new(rect.right() - date_w, rect.top()),
            rect.max,
        );
        (name, size, date)
    }

    fn ui_details(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        let width = ui.available_width().max(200.0);
        let header_h = 25.0;
        let (hrect, _) = ui.allocate_exact_size(Vec2::new(width, header_h), Sense::hover());
        ui.painter().rect_filled(hrect, 0.0, HEADER_BG);
        let (name_c, size_c, date_c) = self.column_rects(hrect.shrink2(Vec2::new(8.0, 0.0)));
        let labels = [
            (SortKey::Name, "Name", name_c, Align2::LEFT_CENTER),
            (SortKey::Size, "Size", size_c, Align2::RIGHT_CENTER),
            (SortKey::Modified, "Modified", date_c, Align2::RIGHT_CENTER),
        ];
        for (key, label, rect, align) in labels {
            let sort_here = self.sort == key;
            let color = if sort_here { TEXT } else { TEXT_DIM };
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
                    .add(egui::Shape::convex_polygon(tri, TEXT_DIM, egui::epaint::PathStroke::NONE));
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
        for x in [size_c.left() - 6.0, date_c.left() - 6.0] {
            ui.painter().line_segment(
                [Pos2::new(x, hrect.top() + 3.0), Pos2::new(x, hrect.bottom() - 3.0)],
                Stroke::new(1.0, SEPARATOR),
            );
        }
        ui.painter().line_segment(
            [Pos2::new(hrect.left(), hrect.bottom()), Pos2::new(hrect.right(), hrect.bottom())],
            Stroke::new(1.0, BORDER),
        );

        let icon_sz = self.icon_size().clamp(16.0, 256.0);
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
                ROW_SEL_ACTIVE
            } else {
                ROW_SEL_INACTIVE
            }
        } else if resp.hovered() {
            ROW_HOVER
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 0.0, bg);
        }
        ui.painter().line_segment(
            [Pos2::new(rect.left() + 4.0, rect.bottom()), Pos2::new(rect.right() - 4.0, rect.bottom())],
            Stroke::new(1.0, SEPARATOR.gamma_multiply(0.55)),
        );

        let (name_c, size_c, date_c) = self.column_rects(rect.shrink2(Vec2::new(8.0, 0.0)));
        // The icon follows the zoom level, as it does in the original's details
        // view: Ctrl+Plus makes the rows grow instead of only changing the label
        // in the status bar.
        let icon_sz = self.icon_size().clamp(16.0, 256.0);
        let icon_rect = Rect::from_center_size(
            Pos2::new(name_c.left() + icon_sz * 0.5 + 2.0, rect.center().y),
            Vec2::splat(icon_sz),
        );
        let mut drew_preview = false;
        if self.previews && entry.is_image() {
            let px = (icon_sz * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px.max(16)) {
                draw_preview(ui.painter(), &tex, icon_rect);
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

        let text_color = TEXT;
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
            let label = theme::elide(ui, &entry.name, max_w, font.clone());
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

        let size_text = if entry.is_dir {
            match entry.children {
                Some(n) => format!("{} item{}", n, if n == 1 { "" } else { "s" }),
                None => "—".to_string(),
            }
        } else {
            fsutil::format_size(entry.size)
        };
        ui.painter().text(
            Pos2::new(size_c.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            size_text,
            FontId::proportional(12.0),
            TEXT_DIM,
        );
        ui.painter().text(
            Pos2::new(date_c.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            fsutil::short_date(entry.modified),
            FontId::proportional(12.0),
            TEXT_DIM,
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
        let icon = self.icon_size().clamp(16.0, 256.0);
        let cell_h = icon_cell_height(icon);
        // Cell width comes from `DolphinItemListView::updateGridSize` @0x26db80:
        // every cell at a given zoom is the same width, sized for the label
        // (exp(zoom/13) * 64 * averageCharWidth/9 + 48, at least exp(zoom/13)*4
        // + the icon). The width of the label therefore decides how many
        // pictures share a row, exactly as in the original.
        let zoom = self.zoom[self.view.index()];
        // `QFontMetrics::averageCharWidth` in the original; 'n' at the label
        // size is the same width to within a pixel for the fonts in use.
        let avg_char = ui
            .painter()
            .layout_no_wrap("n".to_owned(), egui::FontId::proportional(13.0), TEXT)
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
                let aspect = if previews && entry.is_image() {
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
                ROW_SEL_ACTIVE
            } else {
                ROW_SEL_INACTIVE
            }
        } else if resp.hovered() {
            ROW_HOVER
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink(4.0), CornerRadius::same(4), bg);
        }
        let aspect = if self.previews && entry.is_image() {
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
        if self.previews && entry.is_image() {
            // Previews are shown from the smallest zoom level up, as in the
            // original, and the thumbnail is rendered at the icon size times the
            // screen scale so zooming in always gives a sharper, bigger picture.
            let px = (icon * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px) {
                draw_preview(ui.painter(), &tex, icon_rect);
                drew_thumb = true;
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
        let font = FontId::proportional(12.0);
        let mut lines = wrap_label(ui, &entry.name, max_w, font.clone(), if icon >= 32.0 { 2 } else { 1 });
        // A second line holding one or two leftover letters ("…jpg" cut to "g")
        // looks like a mistake; keep the single fuller line instead.
        if lines.len() == 2 && lines[1].chars().count() < 3 {
            lines.truncate(1);
        }
        for (i, line) in lines.iter().enumerate() {
            ui.painter().text(
                Pos2::new(rect.center().x, label_y + 11.0 + i as f32 * 13.0),
                Align2::CENTER_CENTER,
                line,
                font.clone(),
                if selected { Color32::WHITE } else { TEXT },
            );
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
        let icon = self.icon_size().clamp(16.0, 256.0);
        let row_h = (icon + 6.0).max(22.0);
        // Compact cell width from `DolphinItemListView::updateGridSize`
        // @0x26db80: iconSize + 8 + 5 * fontHeight.
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
                ROW_SEL_ACTIVE
            } else {
                ROW_SEL_INACTIVE
            }
        } else if resp.hovered() {
            ROW_HOVER
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink2(Vec2::new(3.0, 1.0)), CornerRadius::same(3), bg);
        }
        let icon = self.icon_size().clamp(16.0, 256.0);
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 9.0 + icon / 2.0, rect.center().y),
            Vec2::splat(icon),
        );
        let mut drew_preview = false;
        if self.previews && entry.is_image() {
            let px = (icon * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px.max(16)) {
                draw_preview(ui.painter(), &tex, icon_rect);
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
        let font = FontId::proportional(12.5);
        let label_x = rect.left() + icon + 18.0;
        let max_w = (rect.right() - label_x - 8.0).max(20.0);
        let label = theme::elide(ui, &entry.name, max_w, font.clone());
        ui.painter().text(
            Pos2::new(label_x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            if selected { Color32::WHITE } else { TEXT },
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
        let current_dir = self.current_dir();
        let entry = self.display_entries().get(idx).cloned();
        if let Some(entry) = entry {
            let selected_paths: Vec<std::path::PathBuf> = if self.sel.contains(&idx) && self.sel.len() > 1 {
                self.selected_paths()
            } else {
                vec![entry.path.clone()]
            };
            let in_trash = self.is_trash();
            let clipboard_has = !vctx.clipboard.paths.is_empty();
            resp.context_menu(|ui| {
                ui.set_min_width(210.0);
                if in_trash {
                    if ui.button("Restore to Former Location").clicked() {
                        vctx.reqs.push(Request::Restore(selected_paths.clone()));
                        ui.close();
                    }
                    if ui.button("Delete Permanently").clicked() {
                        vctx.reqs.push(Request::DeletePermanent(vec![entry.path.clone()]));
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
                if !entry.is_dir && entry.is_image() && ui.button("Open in Picture Viewer").clicked() {
                    vctx.reqs.push(Request::OpenViewer(entry.path.clone(), false));
                    ui.close();
                }
                if !entry.is_dir && entry.is_image() && ui.button("Picture Slideshow").clicked() {
                    vctx.reqs.push(Request::OpenViewer(entry.path.clone(), true));
                    ui.close();
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
                if ui.button("Copy Location").clicked() {
                    ui.ctx().copy_text(entry.path.display().to_string());
                    ui.close();
                }
                ui.separator();
                if ui.button("Move to Trash").clicked() {
                    vctx.reqs.push(Request::Trash(selected_paths.clone()));
                    ui.close();
                }
                if ui.button("Delete Permanently…").clicked() {
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
            TEXT_DIM,
            (rect.width() - 48.0).max(120.0),
        );
        let pos = Pos2::new(center.x - galley.size().x * 0.5, center.y - galley.size().y * 0.5);
        ui.painter().galley(pos, galley, TEXT_DIM);
        if !hint.is_empty() {
            ui.painter().text(
                Pos2::new(center.x, center.y + 22.0),
                Align2::CENTER_CENTER,
                hint,
                FontId::proportional(12.0),
                TEXT_FAINT,
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
    let icon = icon.clamp(16.0, 256.0);
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
    let icon = icon.clamp(16.0, 256.0);
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
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), TEXT))
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
