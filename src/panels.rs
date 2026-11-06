//! The extra panels: Places additions, Information panel, menu, toasts and the
//! dialogs (confirm, properties, bulk rename, duplicates, quick look).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Pos2, Rect, Sense,
    Stroke, Vec2,
};

use crate::fsutil::{self, Entry};
use crate::state::Request;
use crate::theme;
use crate::theme::*;
use crate::{App, Bulk, Confirm, ConfirmAction, Dups, NewFolder, Props, Quick, ShortcutEdit};

fn window_style<'a>(title: &'a str) -> egui::Window<'a> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .frame(
            egui::Frame::window(&egui::Style::default())
                .fill(Color32::from_rgb(43, 47, 51))
                .inner_margin(Margin::same(14))
                .corner_radius(CornerRadius::same(6))
                .stroke(Stroke::new(1.0, BORDER)),
        )
}

impl App {
    pub fn app_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(250.0);
        let dir = self.pane().current_dir();
        if crate::menu_row(ui, "New Window", "Ctrl+N").clicked() {
            crate::spawn_folder_window(&dir);
            ui.close();
        }
        if crate::menu_row(ui, "New Tab", "Ctrl+T").clicked() {
            let d = self.pane().current_dir();
            self.new_tab_public(d);
            ui.close();
        }
        if crate::menu_row(ui, "New Folder…", "F10").clicked() {
            self.new_folder = Some(NewFolder {
                name: "New Folder".to_string(),
                focus: false,
            });
            ui.close();
        }
        if crate::menu_row(ui, "Add Shortcut…", "Ctrl+Shift+S").clicked() {
            self.open_shortcut_editor(None, dir.clone());
            ui.close();
        }
        if crate::menu_row(ui, "Open Terminal Here", "").clicked() {
            fsutil::open_terminal(&dir);
            ui.close();
        }
        ui.separator();
        if crate::menu_row(ui, "Find Duplicates…", "Ctrl+Shift+D").clicked() {
            self.dups = Some(Dups {
                scan: crate::search::DupScan::start(dir.clone()),
            });
            ui.close();
        }
        if crate::menu_row(ui, "Bulk Rename…", "Ctrl+Shift+R").clicked() {
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
            ui.close();
        }
        if crate::menu_row(ui, "Calculate Folder Sizes", "").clicked() {
            let entries = self.pane().entries.clone();
            let dirs: Vec<PathBuf> = entries
                .into_iter()
                .filter(|e| e.is_dir)
                .map(|e| e.path)
                .collect();
            if !dirs.is_empty() {
                self.reqs.push(Request::FolderSize(dirs));
            }
            ui.close();
        }
        ui.separator();
        let mut show_hidden = self.pane().show_hidden;
        if ui.checkbox(&mut show_hidden, "Show Hidden Files").changed() {
            self.pane_mut().show_hidden = show_hidden;
            self.pane_mut().reload();
        }
        let mut previews = self.pane().previews;
        if ui.checkbox(&mut previews, "Show Preview").changed() {
            self.pane_mut().previews = previews;
        }
        let mut split = self.split;
        if ui.checkbox(&mut split, "Split View").changed() {
            if split {
                self.ensure_split();
            } else {
                self.unsplit();
            }
        }
        let mut places = self.places_visible;
        if ui.checkbox(&mut places, "Show Shortcuts Panel").changed() {
            self.places_visible = places;
        }
        let mut info = self.info_panel;
        if ui.checkbox(&mut info, "Show Information Panel").changed() {
            self.info_panel = info;
        }
        ui.separator();
        // Sorting is also reachable here, so it stays available in a window so
        // narrow that the toolbar's sort control gives way to the location bar.
        ui.menu_button("Sort by", |ui| {
            let mut changed = false;
            {
                let pane = self.pane_mut();
                for key in [
                    crate::state::SortKey::Name,
                    crate::state::SortKey::Size,
                    crate::state::SortKey::Type,
                    crate::state::SortKey::Modified,
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
                // Same three comparisons the original's Sort page lists
                // (`FUN_001e5750` @0x1e5750).
                ui.menu_button("Name comparison", |ui| {
                    for mode in [
                        crate::state::NameSort::Natural,
                        crate::state::NameSort::CaseInsensitive,
                        crate::state::NameSort::CaseSensitive,
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
                });
            }
            if changed {
                self.config_dirty = true;
            }
        });
        ui.separator();
        if crate::menu_row(ui, "About", "").clicked() {
            self.toast(&format!("{} — file manager with extra tools", crate::APP_NAME));
            ui.close();
        }
        if crate::menu_row(ui, "Quit", "Ctrl+Q").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // -----------------------------------------------------------------------
    // Information panel
    // -----------------------------------------------------------------------

    pub fn info_panel_ui(&mut self, ctx: &egui::Context) {
        if !self.info_panel {
            return;
        }
        let mut close = false;
        let mut open_props = false;
        let mut open_quick = false;
        let selected: Vec<Entry> = self
            .pane()
            .sel
            .iter()
            .filter_map(|i| self.pane().display_entries().get(*i).cloned())
            .collect();
        let entry = if selected.len() == 1 {
            Some(selected[0].clone())
        } else if selected.is_empty() {
            self.pane().primary_selection().cloned()
        } else {
            None
        };
        let path_for_size = entry.as_ref().map(|e| e.path.clone());

        egui::SidePanel::right("information")
            .resizable(true)
            .default_width(288.0)
            .width_range(220.0..=460.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .inner_margin(Margin::symmetric(10, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Information")
                            .size(12.0)
                            .color(TEXT_DIM),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(20.0, 20.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(3),
                                crate::theme::ROW_HOVER,
                            );
                        }
                        let c = rect.center();
                        let stroke = egui::Stroke::new(1.3, TEXT_DIM);
                        ui.painter().line_segment(
                            [Pos2::new(c.x - 4.0, c.y - 4.0), Pos2::new(c.x + 4.0, c.y + 4.0)],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [Pos2::new(c.x + 4.0, c.y - 4.0), Pos2::new(c.x - 4.0, c.y + 4.0)],
                            stroke,
                        );
                        if resp.on_hover_text("Close (F11)").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(4.0);

                let Some(entry) = entry.clone() else {
                    // `FUN_001f76f0` @0x1f76f0: when more than one item is
                    // selected the panel swaps the per-file rows for the
                    // `dialog-information` icon and the label `%1 item selected`
                    // / `%1 items selected` (string 0x22ed75), and that label
                    // carries no total size — the size of a selection is the
                    // status bar's job (`DolphinView::emitStatusBarText`).
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        let side = 44.0;
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                        let c = rect.center();
                        ui.painter().circle_stroke(
                            c,
                            side * 0.5 - 2.0,
                            Stroke::new(1.6, TEXT_DIM),
                        );
                        ui.painter().text(
                            c,
                            Align2::CENTER_CENTER,
                            "i",
                            FontId::proportional(22.0),
                            TEXT_DIM,
                        );
                        ui.add_space(8.0);
                        let msg = if selected.is_empty() {
                            "No item selected".to_string()
                        } else {
                            format!("{} items selected", selected.len())
                        };
                        ui.label(egui::RichText::new(msg).size(13.0).color(TEXT_DIM));
                    });
                    return;
                };

                let preview_h = 150.0;
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), preview_h), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(33, 37, 40));
                let mut drew = false;
                if entry.is_image() {
                    if let Some(tex) = self.thumbs.get(ui.ctx(), &entry.path, preview_h as u32 * 2) {
                        let scale = (rect.width() / tex.size()[0] as f32)
                            .min(rect.height() / tex.size()[1] as f32)
                            .min(1.0);
                        let size = Vec2::new(
                            tex.size()[0] as f32 * scale,
                            tex.size()[1] as f32 * scale,
                        );
                        ui.painter().image(
                            tex.id(),
                            Rect::from_center_size(rect.center(), size),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        drew = true;
                    }
                }
                if !drew {
                    let icon_rect =
                        Rect::from_center_size(rect.center(), Vec2::splat(80.0));
                    theme::draw_kind_icon_tinted(
                        ui.painter(),
                        icon_rect,
                        entry.kind,
                        entry.broken,
                        entry
                            .tint
                            .filter(|_| entry.is_dir)
                            .map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                    );
                }

                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    let label = if entry.name.chars().count() > 40 {
                        format!("{}…", entry.name.chars().take(40).collect::<String>())
                    } else {
                        entry.name.clone()
                    };
                    ui.label(
                        egui::RichText::new(label)
                            .size(14.0)
                            .strong()
                            .color(TEXT),
                    );
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let size_text = if entry.is_dir {
                    if let Some(size) = self.folder_sizes.get(&entry.path) {
                        fsutil::format_size(*size)
                    } else {
                        let (dirs, files) = crate::search::count_items(&entry.path);
                        format!(
                            "{} item{} ({} folder{}, {} file{})",
                            dirs + files,
                            if dirs + files == 1 { "" } else { "s" },
                            dirs,
                            if dirs == 1 { "" } else { "s" },
                            files,
                            if files == 1 { "" } else { "s" }
                        )
                    }
                } else {
                    fsutil::format_size(entry.size)
                };
                info_row(ui, "Type", &entry.type_label());
                info_row(ui, "Size", &size_text);
                info_row(ui, "Modified", &fsutil::human_date(entry.modified));
                info_row(ui, "Accessed", &fsutil::human_date(entry.accessed));
                info_row(ui, "Created", &fsutil::human_date(entry.created));
                info_row(ui, "Tags", "No tags");
                rating_row(ui);
                comment_row(ui);
                info_row(
                    ui,
                    "Permissions",
                    &fsutil::symbolic_permissions(entry.mode, entry.is_dir),
                );
                if let Some(orig) = &entry.original {
                    info_row(ui, "Originally", &orig.display().to_string());
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Quick Look").clicked() {
                        open_quick = true;
                    }
                    if ui.button("Properties").clicked() {
                        open_props = true;
                    }
                });
                let _ = &path_for_size;
            });

        if close {
            self.info_panel = false;
        }
        if open_props {
            let paths = if selected.len() > 1 {
                selected.iter().map(|e| e.path.clone()).collect()
            } else if let Some(e) = &entry {
                vec![e.path.clone()]
            } else {
                Vec::new()
            };
            if !paths.is_empty() {
                self.props = Some(Props {
                    paths,
                    checksum: None,
                    busy: false,
                    rx: None,
                    folder_size: None,
                });
            }
        }
        if open_quick {
            if let Some(e) = entry {
                self.reqs.push(Request::QuickLook(e.path));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Toast
    // -----------------------------------------------------------------------

    pub fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, when)) = self.toast.clone() else { return };
        let age = when.elapsed();
        if age > Duration::from_secs(4) {
            self.toast = None;
            return;
        }
        let alpha = if age > Duration::from_secs(3) {
            (1.0 - (age.as_secs_f32() - 3.0)) as f32
        } else {
            1.0
        };
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("toast"))
            .fixed_pos(Pos2::new(screen.center().x - 200.0, screen.bottom() - 74.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        30, 34, 38,
                        (235.0 * alpha) as u8,
                    ))
                    .stroke(Stroke::new(1.0, SEPARATOR))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(360.0);
                        ui.label(
                            egui::RichText::new(msg)
                                .size(12.5)
                                .color(TEXT.gamma_multiply(alpha)),
                        );
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    // -----------------------------------------------------------------------
    // Dialogs
    // -----------------------------------------------------------------------

    pub fn dialogs(&mut self, ctx: &egui::Context) {
        self.confirm_dialog(ctx);
        self.props_dialog(ctx);
        self.bulk_dialog(ctx);
        self.dups_dialog(ctx);
        self.quick_dialog(ctx);
        self.new_folder_dialog(ctx);
        self.shortcut_dialog(ctx);
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm.take() else { return };
        let mut action = false;
        let mut cancel = false;
        let mut win_open = true;
        window_style(&confirm.title)
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.label(egui::RichText::new(&confirm.body).size(13.0).color(TEXT));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(90.0, 26.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (label, color) = match &confirm.action {
                            ConfirmAction::Trash(_) => ("Move to Trash", Color32::from_rgb(61, 130, 180)),
                            ConfirmAction::Restore(_) => ("Restore", Color32::from_rgb(61, 130, 180)),
                            _ => ("Delete", Color32::from_rgb(170, 68, 62)),
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label).color(Color32::WHITE),
                                )
                                .fill(color)
                                .min_size(Vec2::new(130.0, 26.0)),
                            )
                            .clicked()
                        {
                            action = true;
                        }
                    });
                });
            });
        if action {
            self.run_confirm(confirm.action);
        } else if !cancel && win_open {
            self.confirm = Some(confirm);
        }
    }

    fn run_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Trash(paths) => match fsutil::move_to_trash(&paths) {
                Ok(n) => {
                    self.toast(&format!(
                        "Trash operation completed. {} item{} moved.",
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                }
                Err(e) => self.toast(&e),
            },
            ConfirmAction::Delete(paths) => {
                let mut errors = Vec::new();
                for p in &paths {
                    if std::fs::remove_dir_all(p).is_err() && std::fs::remove_file(p).is_err() {
                        errors.push(p.display().to_string());
                    }
                }
                if errors.is_empty() {
                    self.toast("Delete operation completed.");
                } else {
                    self.toast(&format!("Could not delete: {}", errors.join(", ")));
                }
            }
            ConfirmAction::EmptyTrash => match fsutil::empty_trash() {
                Ok(()) => self.toast("Trash emptied."),
                Err(e) => self.toast(&e),
            },
            ConfirmAction::Restore(paths) => {
                let mut ok = 0;
                for p in &paths {
                    if let Some(entry) = fsutil::list_trash().into_iter().find(|e| &e.path == p) {
                        if fsutil::restore_trash_entry(&entry).is_ok() {
                            ok += 1;
                        }
                    }
                }
                self.toast(&format!("Restored {} item(s).", ok));
            }
        }
        for tab in self.tabs.iter_mut() {
            for pane in tab.panes.iter_mut() {
                pane.reload();
            }
        }
    }

    fn props_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut props) = self.props.take() else { return };
        let mut win_open = true;
        let mut close = false;
        let mut start_hash = false;
        let mut start_folder_size = false;
        let mut path_to_hash: Option<PathBuf> = None;
        let mut path_for_size: Option<PathBuf> = None;

        window_style("Properties")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                if props.paths.len() == 1 {
                    let entry = Entry::from_path(&props.paths[0]);
                    let tint = crate::foldercolor::read(&entry.path);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                        theme::draw_kind_icon_tinted(
                            ui.painter(),
                            r,
                            entry.kind,
                            entry.broken,
                            tint.map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                        );
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&entry.name).size(14.0).strong());
                            ui.label(
                                egui::RichText::new(entry.type_label())
                                    .size(12.0)
                                    .color(TEXT_DIM),
                            );
                        });
                    });
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    info_row(ui, "Location", &entry.path.parent().map(|p| p.display().to_string()).unwrap_or_default());
                    if entry.is_dir {
                        let name = tint
                            .and_then(crate::foldercolor::name_of)
                            .unwrap_or("Normal");
                        info_row(ui, "Folder colour", name);
                    }
                    info_row(
                        ui,
                        "Size",
                        &match (&props.folder_size, entry.is_dir) {
                            (Some(s), true) => fsutil::format_size(*s),
                            (None, true) => format!("{} (not calculated)", fsutil::format_size(0)),
                            _ => fsutil::format_size(entry.size),
                        },
                    );
                    info_row(ui, "Modified", &fsutil::long_date(entry.modified));
                    info_row(ui, "Accessed", &fsutil::long_date(entry.accessed));
                    info_row(ui, "Created", &fsutil::long_date(entry.created));
                    info_row(
                        ui,
                        "Permissions",
                        &format!(
                            "{} ({})",
                            fsutil::symbolic_permissions(entry.mode, entry.is_dir),
                            fsutil::octal_permissions(entry.mode)
                        ),
                    );
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(md) = std::fs::metadata(&entry.path) {
                        info_row(
                            ui,
                            "Owner",
                            &format!(
                                "{} : {}",
                                fsutil::user_name(md.uid()),
                                fsutil::group_name(md.gid())
                            ),
                        );
                    }
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Checksum")
                            .size(11.0)
                            .color(TEXT_DIM),
                    );
                    ui.horizontal(|ui| {
                        let label = if props.busy {
                            "Computing…".to_string()
                        } else if let Some(hash) = &props.checksum {
                            hash.clone()
                        } else {
                            "Not computed".to_string()
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(label)
                                    .monospace()
                                    .size(10.5)
                                    .color(if props.checksum.is_some() { TEXT } else { TEXT_FAINT }),
                            )
                            .truncate(),
                        );
                        if !entry.is_dir && !props.busy {
                            if ui.small_button("SHA-256").clicked() {
                                start_hash = true;
                                path_to_hash = Some(entry.path.clone());
                            }
                        }
                        if entry.is_dir && !props.busy {
                            if ui.small_button("Calculate size").clicked() {
                                start_folder_size = true;
                                path_for_size = Some(entry.path.clone());
                            }
                        }
                        if let Some(hash) = &props.checksum {
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(hash.clone());
                            }
                        }
                    });
                } else {
                    ui.label(
                        egui::RichText::new(format!("{} items selected", props.paths.len()))
                            .size(14.0)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    let total: u64 = props
                        .paths
                        .iter()
                        .map(|p| std::fs::symlink_metadata(p).map(|m| m.len()).unwrap_or(0))
                        .sum();
                    info_row(ui, "Total size", &fsutil::format_size(total));
                    info_row(ui, "Location", &props.paths[0].parent().map(|p| p.display().to_string()).unwrap_or_default());
                    ui.add_space(6.0);
                    let mut list = String::new();
                    for p in props.paths.iter().take(12) {
                        list.push_str(&format!("{}\n", p.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    if props.paths.len() > 12 {
                        list.push_str("…\n");
                    }
                    ui.add(
                        egui::TextEdit::multiline(&mut list.as_str())
                            .desired_rows(6)
                            .font(egui::TextStyle::Monospace),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });

        if start_hash {
            if let Some(path) = path_to_hash {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = crate::search::sha256_file(&path).ok();
                    let _ = tx.send(result);
                });
                props.rx = Some(rx);
                props.busy = true;
                props.checksum = None;
            }
        }
        if start_folder_size {
            if let Some(path) = path_for_size {
                let tx = self.size_tx.clone();
                let p = path.clone();
                std::thread::spawn(move || {
                    let size = crate::search::dir_size(&p);
                    let _ = tx.send((p, size));
                });
            }
        }
        if let Some(path) = props.paths.first() {
            if let Some(size) = self.folder_sizes.get(path) {
                props.folder_size = Some(*size);
            }
        }
        if close || !win_open {
            return;
        }
        self.props = Some(props);
    }

    fn bulk_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut bulk) = self.bulk.take() else { return };
        let mut win_open = true;
        let mut close = false;
        let mut apply = false;
        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();

        window_style("Bulk Rename")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                ui.label(
                    egui::RichText::new(format!("{} items selected", bulk.paths.len()))
                        .size(12.0)
                        .color(TEXT_DIM),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Pattern:");
                    ui.add(
                        egui::TextEdit::singleline(&mut bulk.pattern)
                            .desired_width(300.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.label("Start at:");
                    let mut start = bulk.start as i32;
                    if ui
                        .add(egui::DragValue::new(&mut start).range(0..=9999))
                        .changed()
                    {
                        bulk.start = start.max(0) as u32;
                    }
                });
                ui.label(
                    egui::RichText::new("Tokens: {name} {n} {ext} {date}")
                        .size(10.5)
                        .color(TEXT_FAINT),
                );
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (i, path) in bulk.paths.iter().enumerate() {
                            let old = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                            let new = expand_pattern(&bulk.pattern, path, bulk.start as u64 + i as u64, &date);
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(old).size(11.5).color(TEXT_DIM),
                                    )
                                    .truncate(),
                                );
                                ui.label(egui::RichText::new("→").size(11.5).color(TEXT_FAINT));
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(new).size(11.5).color(TEXT),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                    });
                if let Some(err) = &bulk.error {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(err).size(11.5).color(Color32::from_rgb(220, 120, 110)));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Rename").clicked() {
                            apply = true;
                        }
                    });
                });
            });

        if apply {
            match bulk_rename(&bulk.paths, &bulk.pattern, bulk.start, &date) {
                Ok(n) => {
                    self.toast(&format!(
                        "Renamed {} item{}",
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                    close = true;
                }
                Err(e) => bulk.error = Some(e),
            }
            for tab in self.tabs.iter_mut() {
                for pane in tab.panes.iter_mut() {
                    pane.reload();
                }
            }
        }
        if close || !win_open {
            return;
        }
        self.bulk = Some(bulk);
    }

    fn dups_dialog(&mut self, ctx: &egui::Context) {
        let Some(dups) = self.dups.take() else { return };
        let mut win_open = true;
        let mut close = false;
        let mut rescan = false;
        let mut trash_files: Vec<PathBuf> = Vec::new();

        if dups.scan.scanning {
            ctx.request_repaint_after(Duration::from_millis(300));
        }

        window_style("Find Duplicates")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(620.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Scanning {}", dups.scan.root.display()))
                            .size(11.0)
                            .color(TEXT_DIM),
                    );
                });
                ui.add_space(6.0);
                if dups.scan.scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(format!(
                                "{} files examined…",
                                dups.scan.scanned
                            ))
                            .size(12.0)
                            .color(TEXT_DIM),
                        );
                    });
                } else if dups.scan.groups.is_empty() {
                    ui.label(
                        egui::RichText::new("No duplicates found.")
                            .size(13.0)
                            .color(TEXT_DIM),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} groups, {} could be freed",
                            dups.scan.groups.len(),
                            fsutil::format_size(dups.scan.wasted())
                        ))
                        .size(12.0)
                        .color(TEXT),
                    );
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (gi, group) in dups.scan.groups.iter().enumerate() {
                            egui::Frame::new()
                                .fill(Color32::from_rgb(37, 41, 45))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(Margin::symmetric(8, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} × {}",
                                                group.files.len(),
                                                fsutil::format_size(group.size)
                                            ))
                                            .size(12.0)
                                            .strong(),
                                        );
                                        ui.label(
                                            egui::RichText::new(&group.hash[..12])
                                                .monospace()
                                                .size(10.0)
                                                .color(TEXT_FAINT),
                                        );
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button("Keep first, trash the rest")
                                                    .clicked()
                                                {
                                                    let keep = group.files[0].clone();
                                                    let rest: Vec<PathBuf> = group
                                                        .files
                                                        .iter()
                                                        .filter(|p| **p != keep)
                                                        .cloned()
                                                        .collect();
                                                    trash_files.extend(rest);
                                                }
                                            },
                                        );
                                    });
                                    for (i, f) in group.files.iter().enumerate() {
                                        let color = if i == 0 { TEXT } else { TEXT_DIM };
                                        ui.label(
                                            egui::RichText::new(f.display().to_string())
                                                .size(10.5)
                                                .color(color),
                                        );
                                    }
                                });
                            ui.add_space(4.0);
                            let _ = gi;
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if dups.scan.scanning {
                        if ui.button("Stop").clicked() {
                            close = true;
                        }
                    } else if ui.button("Scan again").clicked() {
                        rescan = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });

        if !trash_files.is_empty() {
            match fsutil::move_to_trash(&trash_files) {
                Ok(n) => self.toast(&format!(
                    "Moved {} duplicate{} to the Trash",
                    n,
                    if n == 1 { "" } else { "s" }
                )),
                Err(e) => self.toast(&e),
            }
            for tab in self.tabs.iter_mut() {
                for pane in tab.panes.iter_mut() {
                    pane.reload();
                }
            }
        }
        if rescan {
            let root = dups.scan.root.clone();
            self.dups = Some(Dups {
                scan: crate::search::DupScan::start(root),
            });
            return;
        }
        if close || !win_open {
            return;
        }
        self.dups = Some(dups);
    }

    fn quick_dialog(&mut self, ctx: &egui::Context) {
        let Some(quick) = self.quick.take() else { return };
        let mut win_open = true;
        let entry = Entry::from_path(&quick.path);
        let screen = ctx.screen_rect();
        let max_h = screen.height() * 0.72;
        egui::Window::new("Quick Look")
            .open(&mut win_open)
            .collapsible(false)
            .resizable(true)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(760.0)
            .default_height(560.0)
            .max_height(max_h)
            .frame(
                egui::Frame::window(&egui::Style::default())
                    .fill(Color32::from_rgb(26, 29, 32))
                    .inner_margin(Margin::same(12))
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    let tint = crate::foldercolor::read(&entry.path);
                    theme::draw_kind_icon_tinted(
                        ui.painter(),
                        r,
                        entry.kind,
                        false,
                        tint.map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&entry.name).size(13.0).strong(),
                        )
                        .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} • {}",
                                entry.type_label(),
                                if entry.is_dir {
                                    "folder".to_string()
                                } else {
                                    fsutil::format_size(entry.size)
                                }
                            ))
                            .size(11.0)
                            .color(TEXT_FAINT),
                        );
                    });
                });
                ui.separator();
                if entry.is_dir {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::hover());
                        theme::draw_kind_icon(ui.painter(), r, fsutil::Kind::Folder, false);
                        ui.add_space(12.0);
                        let (dirs, files) = crate::search::count_items(&entry.path);
                        ui.label(
                            egui::RichText::new(format!("{} folders, {} files", dirs, files))
                                .size(13.0)
                                .color(TEXT_DIM),
                        );
                    });
                } else if entry.is_image() {
                    let avail = ui.available_size();
                    let (rect, _) = ui.allocate_exact_size(avail, Sense::hover());
                    if let Some(tex) = self.thumbs.get(ui.ctx(), &entry.path, 1400) {
                        let scale = (rect.width() / tex.size()[0] as f32)
                            .min(rect.height() / tex.size()[1] as f32)
                            .min(1.0);
                        let size = Vec2::new(tex.size()[0] as f32 * scale, tex.size()[1] as f32 * scale);
                        ui.painter().image(
                            tex.id(),
                            Rect::from_center_size(rect.center(), size),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(120.0);
                            ui.label(egui::RichText::new("Loading preview…").color(TEXT_DIM));
                        });
                    }
                } else if let Some(text) = &quick.text {
                    let mut text = text.as_str();
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.label(
                            egui::RichText::new("No preview available for this file type")
                                .size(13.0)
                                .color(TEXT_DIM),
                        );
                    });
                }
            });
        if !win_open {
            return;
        }
        self.quick = Some(quick);
    }

    fn new_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut nf) = self.new_folder.take() else { return };
        let mut win_open = true;
        let mut create = false;
        let mut close = false;
        window_style("Create Folder")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut nf.name)
                        .desired_width(f32::INFINITY)
                        .hint_text("Folder name"),
                );
                if !nf.focus {
                    resp.request_focus();
                    theme::select_all_text(ctx, resp.id, nf.name.chars().count());
                    nf.focus = true;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Create").clicked() {
                            create = true;
                        }
                    });
                });
            });
        if create {
            let dir = self.pane().current_dir();
            let name = nf.name.trim().to_string();
            if name.is_empty() {
                nf.focus = true;
                self.new_folder = Some(nf);
                return;
            }
            let target = dir.join(&name);
            if target.exists() {
                self.toast("An item with that name already exists.");
                nf.focus = true;
                self.new_folder = Some(nf);
                return;
            }
            match std::fs::create_dir(&target) {
                Ok(()) => {
                    self.pane_mut().reload();
                    if let Some(idx) = self.pane().entries.iter().position(|e| e.path == target) {
                        self.pane_mut().select_single(idx);
                        self.pane_mut().scroll_to = Some(idx);
                        self.pane_mut().rename = Some(crate::state::RenameState {
                            index: idx,
                            text: name.clone(),
                            focus: false,
                        });
                    }
                    self.toast("Folder created.");
                }
                Err(e) => self.toast(&format!("Could not create folder: {}", e)),
            }
            return;
        }
        if close || !win_open {
            return;
        }
        self.new_folder = Some(nf);
    }

    fn shortcut_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.shortcut_edit.take() else {
            return;
        };
        let mut win_open = true;
        let mut save = false;
        let mut close = false;
        let renaming = edit.original.is_some();
        let suggestions: Vec<(&str, PathBuf)> = vec![
            ("This folder", self.pane().current_dir()),
            ("Home", fsutil::home_dir()),
            ("Documents", fsutil::xdg_dir("DOCUMENTS", "Documents")),
            ("Downloads", fsutil::xdg_dir("DOWNLOAD", "Downloads")),
            ("Pictures", fsutil::xdg_dir("PICTURES", "Pictures")),
        ];
        window_style(if renaming {
            "Rename Shortcut"
        } else {
            "Add Shortcut"
        })
        .open(&mut win_open)
        .show(ctx, |ui| {
            ui.set_min_width(430.0);
            ui.label(
                egui::RichText::new("Shortcuts stay in the panel on the left, so one click takes you back here.")
                    .size(11.5)
                    .color(TEXT_DIM),
            );
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Name").size(11.0).color(TEXT_DIM));
            let name_resp = ui.add(
                egui::TextEdit::singleline(&mut edit.name)
                    .desired_width(f32::INFINITY)
                    .hint_text("Shown in the panel"),
            );
            if !edit.focus {
                name_resp.request_focus();
                theme::select_all_text(ctx, name_resp.id, edit.name.chars().count());
                edit.focus = true;
            }
            if name_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Folder").size(11.0).color(TEXT_DIM));
            let path_resp = ui.add(
                egui::TextEdit::singleline(&mut edit.path)
                    .desired_width(f32::INFINITY)
                    .hint_text("/home/you/Documents"),
            );
            if path_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (label, path) in &suggestions {
                    if ui.small_button(*label).clicked() {
                        edit.path = path.display().to_string();
                        if edit.name.trim().is_empty() {
                            edit.name = path
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Shortcut".to_string());
                        }
                    }
                }
            });
            if let Some(err) = &edit.error {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(err)
                        .size(11.5)
                        .color(Color32::from_rgb(214, 120, 112)),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(90.0, 26.0)))
                    .clicked()
                {
                    close = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(if renaming { "Save" } else { "Add Shortcut" })
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(61, 130, 180))
                            .min_size(Vec2::new(130.0, 26.0)),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        });
        if save {
            self.shortcut_edit = Some(edit);
            self.save_shortcut();
            return;
        }
        if close || !win_open {
            return;
        }
        self.shortcut_edit = Some(edit);
    }
}

fn rating_row(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(84.0, 16.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new("Rating:")
                        .size(11.0)
                        .color(TEXT_DIM),
                );
            },
        );
        ui.add_space(6.0);
        let width = ui.available_width().min(76.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 14.0), Sense::hover());
        let painter = ui.painter();
        for i in 0..5 {
            let center = Pos2::new(rect.left() + 7.0 + i as f32 * 15.0, rect.center().y);
            let mut points = Vec::with_capacity(10);
            for k in 0..10 {
                let angle = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                let r = if k % 2 == 0 { 6.4 } else { 2.7 };
                points.push(Pos2::new(
                    center.x + r * angle.cos(),
                    center.y + r * angle.sin(),
                ));
            }
            painter.add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.0, TEXT_DIM),
            ));
        }
    });
    ui.add_space(2.0);
}

fn comment_row(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(84.0, 16.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new("Comment:")
                        .size(11.0)
                        .color(TEXT_DIM),
                );
            },
        );
        ui.add_space(6.0);
        let width = ui.available_width().min(140.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(3),
            Color32::from_rgb(28, 32, 35),
            egui::Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
    });
    ui.add_space(2.0);
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(84.0, 18.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(format!("{}:", label))
                        .size(11.0)
                        .color(TEXT_DIM),
                );
            },
        );
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(egui::RichText::new(value).size(11.5).color(TEXT))
                .wrap(),
        );
    });
    ui.add_space(1.0);
}

fn expand_pattern(pattern: &str, path: &Path, n: u64, date: &str) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    pattern
        .replace("{name}", &stem)
        .replace("{ext}", &ext)
        .replace("{n}", &format!("{:02}", n))
        .replace("{date}", date)
}

pub(crate) fn bulk_rename(paths: &[PathBuf], pattern: &str, start: u32, date: &str) -> Result<usize, String> {
    // Validate first so a collision never leaves a half-renamed selection.
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, path) in paths.iter().enumerate() {
        let name = expand_pattern(pattern, path, start as u64 + i as u64, date);
        if name.is_empty() || name.contains('/') {
            return Err(format!("'{}' is not a valid file name.", name));
        }
        let target = path.with_file_name(&name);
        if target != *path && target.exists() {
            return Err(format!("'{}' already exists.", name));
        }
        if !seen.insert(target.clone()) {
            return Err(format!("Two files would be called '{}'.", name));
        }
        plan.push((path.clone(), target));
    }
    let mut done = 0;
    for (from, to) in plan {
        if from == to {
            continue;
        }
        std::fs::rename(&from, &to)
            .map_err(|e| format!("Could not rename {}: {}", from.display(), e))?;
        done += 1;
    }
    Ok(done)
}
