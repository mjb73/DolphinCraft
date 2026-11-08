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
use crate::findtext;
use crate::copymove;
use crate::space;
use crate::theme;
use crate::{App, Bulk, Confirm, ConfirmAction, Dups, NewFolder, Props, Quick, ShortcutEdit};

/// A quiet heading inside the app menu, so its rows read as groups ("New",
/// "Tools", "View", "Help") instead of one long list.
fn menu_group(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(10.5).color(theme::text_faint()));
}

fn window_style<'a>(title: &'a str) -> egui::Window<'a> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .frame(
            egui::Frame::window(&egui::Style::default())
                .fill(theme::c(43, 47, 51))
                .inner_margin(Margin::same(14))
                .corner_radius(CornerRadius::same(6))
                .stroke(Stroke::new(1.0, theme::border())),
        )
}

impl App {
    pub fn app_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(250.0);
        let dir = self.pane().current_dir();
        menu_group(ui, "New");
        if crate::menu_row(ui, "New Window", "Ctrl+N").clicked() {
            crate::spawn_folder_window(&dir);
            ui.close();
        }
        if crate::menu_row(ui, "New Tab", "Ctrl+T").clicked() {
            let d = self.pane().current_dir();
            self.new_tab_public(d);
            ui.close();
        }
        // The original's own row, under File: it lists the folders of the tabs
        // closed since the window opened, newest first ("Recently Closed Tabs",
        // 0x12d8ff in /usr/bin/dolphin, and Ctrl+Shift+T for the last one).
        {
            let list: Vec<PathBuf> = self.closed_tabs.iter().rev().cloned().collect();
            ui.add_enabled_ui(!list.is_empty(), |ui| {
                ui.menu_button("Recently Closed Tabs", |ui| {
                    ui.set_min_width(240.0);
                    for (i, path) in list.iter().enumerate() {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string());
                        let hint = if i == 0 { "Ctrl+Shift+T" } else { "" };
                        if crate::menu_row(ui, &name, hint)
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            self.reqs.push(Request::ReopenClosedTab(i));
                            ui.close();
                        }
                    }
                });
            });
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
        menu_group(ui, "Tools");
        // The original's own row, first in its Tools menu: a `manage_disk_space`
        // action labelled "Manage Disk Space Usage" (string 0x22d931, icon
        // `filelight`) whose submenu only launches Filelight. Here it opens the
        // window this build carries (see `space.rs`), so the sentence in the
        // free-space tooltip — "Press to manage disk space usage." — is true
        // without anything to install.
        if crate::menu_row(ui, "Manage Disk Space Usage…", "").clicked() {
            self.reqs.push(Request::SpaceUsage(dir.clone()));
            ui.close();
        }
        // Added feature: the two archive rows. They act on what is marked, and
        // fall back to the folder on show when nothing is — a person who has
        // not marked anything usually means "this folder".
        if crate::menu_row(ui, "Compress…", "").clicked() {
            let paths = self.pane().selected_paths();
            self.reqs.push(Request::Compress(paths, dir.clone()));
            ui.close();
        }
        // The first marked archive, if any is marked: Extract acts on one file.
        let archive: Option<PathBuf> = self
            .pane()
            .selected_paths()
            .into_iter()
            .find(|p| crate::archive::is_archive(p));
        ui.add_enabled_ui(archive.is_some(), |ui| {
            if crate::menu_row(ui, "Extract…", "").clicked() {
                if let Some(path) = archive.clone() {
                    self.reqs.push(Request::Extract(path));
                }
                ui.close();
            }
        });
        // Added this round: *Resize Image…* — one number, in megapixels, and the
        // picture keeps its shape. It works on the pictures in the selection, so
        // the row is greyed out with the reason in its tooltip when there are
        // none.
        {
            let pictures: Vec<PathBuf> = self
                .pane()
                .selected_paths()
                .into_iter()
                .filter(|p| !p.is_dir() && crate::imageresize::supported(p))
                .collect();
            let has = !pictures.is_empty();
            let mut resize_now = false;
            ui.add_enabled_ui(has, |ui| {
                let resp = crate::menu_row(ui, "Resize Image…", "");
                if !has {
                    resp.clone()
                        .on_hover_text("Mark the pictures to resize first.");
                }
                if resp.clicked() {
                    resize_now = true;
                    ui.close();
                }
            });
            if resize_now {
                self.reqs.push(Request::Resize(pictures));
            }
        }
        // Added feature: the original's two Other View actions, and the two
        // rows that copy or move into a folder of your choosing. The Other View
        // rows are greyed out with the original's own sentence for the reason,
        // in the order its `updateActions` tests them (`copymove::verdict`).
        {
            let dest = self.other_dir();
            let selected = self.pane().selected_paths();
            let mut fire: Option<Request> = None;
            for mode in [copymove::Mode::Copy, copymove::Mode::Move] {
                let verdict = copymove::verdict(
                    mode,
                    &copymove::Facts {
                        split: self.split && dest.is_some(),
                        selected: selected.len(),
                        already_there: dest
                            .as_ref()
                            .map(|d| copymove::already_there(d, &selected))
                            .unwrap_or(false),
                        dest_writable: dest.as_ref().map(|d| copymove::writable(d)).unwrap_or(false),
                        may_move: copymove::may_move(&selected),
                    },
                );
                let enabled = verdict.ok;
                let mut clicked = false;
                ui.add_enabled_ui(enabled, |ui| {
                    let resp = crate::menu_row(ui, mode.other_label(), mode.other_shortcut());
                    let hint = verdict
                        .why
                        .clone()
                        .unwrap_or_else(|| mode.whats_this().to_string());
                    resp.clone().on_hover_text(hint);
                    clicked = resp.clicked();
                });
                if clicked {
                    fire = Some(Request::CopyToOther(mode == copymove::Mode::Move));
                }
            }
            for mode in [copymove::Mode::Copy, copymove::Mode::Move] {
                let mut clicked = false;
                ui.add_enabled_ui(!selected.is_empty(), |ui| {
                    let resp = crate::menu_row(ui, mode.chooser_label(), "");
                    let hint = if selected.is_empty() {
                        format!("Mark the items to {} first.", mode.lower())
                    } else {
                        format!(
                            "{} the marked items into a folder you choose — nothing is overwritten",
                            mode.word()
                        )
                    };
                    resp.clone().on_hover_text(hint);
                    clicked = resp.clicked();
                });
                if clicked {
                    fire = Some(Request::OpenCopyChooser(mode));
                }
            }
            if let Some(req) = fire {
                self.reqs.push(req);
                ui.close();
            }
        }
        // Added feature: *Find in Files* — the content modes the original's
        // find bar names but answers elsewhere.
        if crate::menu_row(ui, "Find in Files…", "Ctrl+Shift+F").clicked() {
            self.findtext = Some(findtext::FindText::start(
                dir.clone(),
                findtext::Options::new(String::new()),
            ));
            ui.close();
        }
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
        menu_group(ui, "View");
        let mut show_hidden = self.pane().show_hidden;
        if ui.checkbox(&mut show_hidden, "Show Hidden Files").changed() {
            self.pane_mut().show_hidden = show_hidden;
            self.pane_mut().reload();
            self.config_dirty = true;
        }
        let mut previews = self.pane().previews;
        if ui.checkbox(&mut previews, "Show Preview").changed() {
            self.pane_mut().previews = previews;
            self.config_dirty = true;
        }
        // Three settings the original keeps on two of its own pages, together
        // here. Each is the original's own key and range:
        // `GeneralSettings` `ShowZoomSlider` (the status-bar slider, off by
        // default there), `[PreviewSettings] MaximumSize` (previews only for
        // files below N MiB, 0 = any size) and
        // `ContentDisplaySettings` `RecursiveDirectorySizeLimit` (how deep
        // "size of contents" walks, 1..20).
        ui.menu_button("Previews and sizes", |ui| {
            ui.set_min_width(250.0);
            let mut zoom_slider = self.config.show_zoom_slider;
            if ui
                .checkbox(&mut zoom_slider, "Show zoom slider")
                .on_hover_text(
                    "The original has this off in a fresh profile and switches it \
                     on from the status bar's own menu",
                )
                .changed()
            {
                self.config.show_zoom_slider = zoom_slider;
                self.config_dirty = true;
            }
            ui.separator();
            let mut mib = self.config.preview_max_mib;
            ui.horizontal(|ui| {
                ui.label("Previews for files below");
                if ui
                    .add(
                        egui::DragValue::new(&mut mib)
                            .range(0..=999)
                            .suffix(" MiB"),
                    )
                    .changed()
                {
                    self.config.preview_max_mib = mib;
                    self.thumbs.set_max_mib(mib);
                    self.config_dirty = true;
                }
            });
            ui.label(
                egui::RichText::new(if mib == 0 {
                    "0 = files of any size"
                } else {
                    "bigger files show their type icon"
                })
                .size(10.5)
                .color(theme::text_faint()),
            );
            let mut depth = self.config.size_limit;
            ui.horizontal(|ui| {
                ui.label("Size of contents, up to");
                if ui
                    .add(
                        egui::DragValue::new(&mut depth)
                            .range(
                                crate::search::SIZE_LIMIT_MIN..=crate::search::SIZE_LIMIT_MAX,
                            )
                            .suffix(" folders"),
                    )
                    .changed()
                {
                    self.config.size_limit = depth;
                    self.config_dirty = true;
                }
            });
        });
        // Added feature: the picture gallery, and its viewer for whichever
        // picture is selected.
        let mut pictures = self.pane().pictures;
        if ui
            .checkbox(&mut pictures, "Pictures (Gallery)")
            .on_hover_text("Thumbnails of this folder at 50–400 px, with a viewer and a slideshow (F4)")
            .changed()
        {
            self.pane_mut().set_pictures(pictures);
            self.config.pictures = pictures;
            self.config_dirty = true;
        }
        let picture = self
            .pane()
            .primary_selection()
            .filter(|e| !e.is_dir && e.is_image())
            .map(|e| e.path.clone());
        if let Some(path) = picture {
            if crate::menu_row(ui, "Open in Picture Viewer", "C").clicked() {
                self.reqs.push(Request::OpenViewer(path.clone(), false));
                ui.close();
            }
            if crate::menu_row(ui, "Slideshow from Here", "Space").clicked() {
                self.reqs.push(Request::OpenViewer(path, true));
                ui.close();
            }
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
        // Added feature: the *Network* group can leave the panel, heading and
        // all, for a desktop with no shares. The switch is here rather than on
        // the group's own heading because a hidden group has no heading to
        // click, and a setting with no way back is a trap.
        let mut network = self.config.network_visible;
        if ui
            .checkbox(&mut network, "Show Network Shared Folders")
            .on_hover_text("The Network group in the panel: the shares this desktop has mounted")
            .changed()
        {
            self.config.network_visible = network;
            self.config_dirty = true;
        }
        // Added feature, asked for by the person: dark and light, from the menu.
        // The whole window is repainted from the palette, so egui's own widget
        // colours have to be set again as well as the switch itself.
        let mut light = self.config.light;
        if ui
            .checkbox(&mut light, "Light Mode")
            .on_hover_text("Switch the whole window between the dark and light palette")
            .changed()
        {
            self.config.light = light;
            self.config_dirty = true;
            crate::theme::set_light(light);
            crate::theme::apply_style(ui.ctx());
            ui.ctx().request_repaint();
        }
        let mut info = self.info_panel;
        if ui.checkbox(&mut info, "Show Information Panel").changed() {
            self.info_panel = info;
            self.config_dirty = true;
        }
        ui.separator();
        // The original's Startup page offers "Folders, tabs, and window state
        // from last time" (0x137dc8) and saves it in
        // `DolphinMainWindow::slotSaveSession`. Everything else on this page is
        // always remembered; this switch is only about the tabs, because a window
        // that reopens seven tabs is not everyone's wish.
        menu_group(ui, "Starting");
        let mut reopen = self.config.restore_session;
        if ui
            .checkbox(&mut reopen, "Reopen the tabs I had open")
            .on_hover_text(
                "Sorting, view, zoom, panel widths and window size are always remembered",
            )
            .changed()
        {
            self.config.restore_session = reopen;
            self.config_dirty = true;
        }
        // Added feature: a plain statement of what is kept, and one row to
        // forget it. Everything in this window is written as it is chosen, so
        // the person can see where it went and undo it in a single click.
        ui.label(
            egui::RichText::new("Saved as you go: sort, view, zoom, panels, colours, pictures and the window's size and place.")
                .size(10.5)
                .color(theme::text_faint()),
        );
        if crate::menu_row(ui, "Forget remembered settings", "").clicked() {
            self.reqs.push(Request::ForgetSettings);
            ui.close();
        }
        ui.separator();
        // The original keeps these three on its Confirmations page, one switch
        // per destructive action (`ConfirmTrash`, `ConfirmDelete`,
        // `ConfirmEmptyTrash`). Here the two that the person asked to be
        // question-free start off, and the one that cannot be undone starts on.
        menu_group(ui, "Deleting");
        let mut ask_trash = self.config.confirm_trash;
        if ui
            .checkbox(&mut ask_trash, "Ask before moving to the Trash")
            .on_hover_text("Off: Delete on a marked selection moves it at once, with Undo")
            .changed()
        {
            self.config.confirm_trash = ask_trash;
            self.config_dirty = true;
        }
        let mut ask_delete = self.config.confirm_delete;
        if ui
            .checkbox(&mut ask_delete, "Ask before deleting permanently")
            .on_hover_text("Off: Shift+Delete on a marked selection deletes it at once")
            .changed()
        {
            self.config.confirm_delete = ask_delete;
            self.config_dirty = true;
        }
        let mut ask_empty = self.config.confirm_empty_trash;
        if ui
            .checkbox(&mut ask_empty, "Ask before emptying the Trash")
            .changed()
        {
            self.config.confirm_empty_trash = ask_empty;
            self.config_dirty = true;
        }
        let undo_ready = self
            .undo_trash
            .as_ref()
            .map(|u| u.at.elapsed().as_secs_f32() < crate::UNDO_SECS)
            .unwrap_or(false);
        // The Trash's own action, in the original's own words and with its own
        // rule: the row is there in the Trash view and greyed out while there is
        // nothing in it (`FUN_0018db20`).
        let in_trash = self.pane().is_trash();
        let trash_items = if in_trash {
            self.pane().entries.len()
        } else {
            0
        };
        if in_trash {
            let mut empty = false;
            ui.add_enabled_ui(trash_items > 0, |ui| {
                if crate::menu_row(ui, crate::EMPTY_TRASH_LABEL, "").clicked() {
                    empty = true;
                }
            });
            if empty {
                self.reqs.push(Request::EmptyTrash);
                ui.close();
            }
        }
        ui.add_enabled_ui(undo_ready, |ui| {
            if crate::menu_row(ui, "Undo Move to Trash", "Ctrl+Z").clicked() && undo_ready {
                self.undo_last_trash();
                ui.close();
            }
        });
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
                        changed |= pane.choose_sort(key);
                        ui.close();
                    }
                }
                ui.separator();
                // The original's own pair under the key list
                // (`DolphinViewActionHandler::createActions` @0x28e380).
                if ui.selectable_label(pane.asc, "Ascending").clicked() {
                    changed |= pane.set_sort_dir(false);
                    ui.close();
                }
                if ui.selectable_label(!pane.asc, "Descending").clicked() {
                    changed |= pane.set_sort_dir(true);
                    ui.close();
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
                // Every one of these is remembered for the next start.
                self.config_dirty = true;
            }
        });
        ui.separator();
        menu_group(ui, "Help");
        if crate::menu_row(ui, "What the Two Buttons Do…", "F1").clicked() {
            self.guide_open = true;
            ui.close();
        }
        if crate::menu_row(ui, "About", "").clicked() {
            self.toast(&crate::about_line());
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
        // A prompt button in this panel reports back here and is shown in the
        // status line once the panel has finished drawing.
        let mut prompt_message: Option<String> = None;

        egui::SidePanel::right("information")
            .resizable(true)
            .default_width(288.0)
            .width_range(220.0..=460.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(Margin::symmetric(10, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Information")
                            .size(12.0)
                            .color(theme::text_dim()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(20.0, 20.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(3),
                                crate::theme::row_hover(),
                            );
                        }
                        let c = rect.center();
                        let stroke = egui::Stroke::new(1.3, theme::text_dim());
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

                // The panel's rows run past the bottom of a laptop window once a
                // picture carries a prompt, so the whole body scrolls; the title
                // and its close button stay put above it.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
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
                                Stroke::new(1.6, theme::text_dim()),
                            );
                            ui.painter().text(
                                c,
                                Align2::CENTER_CENTER,
                                "i",
                                FontId::proportional(22.0),
                                theme::text_dim(),
                            );
                            ui.add_space(8.0);
                            let msg = if selected.is_empty() {
                                "No item selected".to_string()
                            } else {
                                format!("{} items selected", selected.len())
                            };
                            ui.label(egui::RichText::new(msg).size(13.0).color(theme::text_dim()));
                        });
                        return;
                    };

                    let preview_h = 150.0;
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), preview_h), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(4), theme::c(33, 37, 40));
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
                                .map(|c| theme::c(c[0], c[1], c[2])),
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
                                .color(theme::text()),
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
                    // Added feature, adopted from the image browser's dimension
                    // caption: the pixel size, the ratio and the megapixels of a
                    // picture, read from its header rather than by decoding it.
                    if !entry.is_dir && entry.is_image() {
                        if let Some((w, h)) = crate::search::probe_image_size(&entry.path) {
                            info_row(ui, "Dimensions", &format!("{} × {} px", w, h));
                            info_row(
                                ui,
                                "Aspect ratio",
                                &format!(
                                    "{} ({:.2}:1)",
                                    crate::gallery::ratio_label(w, h),
                                    w as f32 / h.max(1) as f32
                                ),
                            );
                            info_row(ui, "Pixels", &crate::gallery::megapixels(w, h));
                        }
                    }
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
                    // Added feature: the prompt a picture was made with, read from
                    // the picture's own metadata. The pixel-size rows above already
                    // told the person how big it is, so this block is just the
                    // recipe.
                    if entry.is_image() {
                        ui.add_space(4.0);
                        let path = entry.path.clone();
                        if !self.image_meta.contains_key(&path) {
                            let meta = crate::imagemeta::read(&path);
                            self.image_meta.insert(path.clone(), meta);
                        }
                        let meta = self.image_meta.get(&path).and_then(|m| m.as_ref());
                        // Added feature: the prompt block folds away behind its
                        // title row; the choice is the person's and is kept.
                        let mut prompt_open = self.config.prompt_open;
                        let shown = crate::gallery::prompt_panel(ui, meta, Some(&path), &mut prompt_open);
                        if prompt_open != self.config.prompt_open {
                            self.config.prompt_open = prompt_open;
                            self.config_dirty = true;
                        }
                        if let Some(msg) = shown.message {
                            prompt_message = Some(msg);
                        }
                        ui.add_space(4.0);
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
        if let Some(msg) = prompt_message {
            self.toast(&msg);
        }
    }

    // -----------------------------------------------------------------------
    // Toast
    // -----------------------------------------------------------------------

    pub fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, when)) = self.toast.clone() else { return };
        let age = when.elapsed();
        // A toast that still has an offer attached lives longer, so the way back
        // is on screen long enough to be used.
        let undo_live = self
            .undo_trash
            .as_ref()
            .map(|u| u.at.elapsed() < Duration::from_secs_f32(crate::UNDO_SECS))
            .unwrap_or(false);
        let life = if undo_live {
            Duration::from_secs_f32(crate::UNDO_SECS)
        } else {
            Duration::from_secs(4)
        };
        if age > life {
            // Only the offer's time on screen runs out. The record of what was
            // moved stays, so Ctrl+Z still brings it back later.
            self.toast = None;
            return;
        }
        let fade_from = life.as_secs_f32() - 1.0;
        let alpha = if age.as_secs_f32() > fade_from {
            (life.as_secs_f32() - age.as_secs_f32()).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        let mut undo_clicked = false;
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("toast"))
            .fixed_pos(Pos2::new(screen.center().x - 200.0, screen.bottom() - 74.0))
            .show(ctx, |ui| {
                // The toast follows the palette: a dark card in dark mode, a light
                // one in light mode, so its own text (in the palette's text colour)
                // is readable in both.
                let toast_bg = theme::c(30, 34, 38);
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        toast_bg.r(),
                        toast_bg.g(),
                        toast_bg.b(),
                        (235.0 * alpha) as u8,
                    ))
                    .stroke(Stroke::new(1.0, theme::separator()))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(360.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(msg)
                                    .size(12.5)
                                    .color(theme::text().gamma_multiply(alpha)),
                            );
                            if undo_live {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let button = egui::Button::new(
                                        egui::RichText::new("Undo")
                                            .size(12.5)
                                            .color(theme::on_band()),
                                    )
                                    .fill(theme::c(61, 130, 180))
                                    .min_size(Vec2::new(64.0, 20.0));
                                    if ui
                                        .add(button)
                                        .on_hover_text("Put the items back where they were")
                                        .clicked()
                                    {
                                        undo_clicked = true;
                                    }
                                });
                            }
                        });
                    });
            });
        if undo_clicked {
            self.undo_last_trash();
        }
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
        self.open_with_dialog(ctx);
        self.space_dialog(ctx);
        self.findtext_dialog(ctx);
        self.compress_dialog(ctx);
        self.extract_dialog(ctx);
        self.copy_dialog(ctx);
        self.resize_dialog(ctx);
        self.shortcut_dialog(ctx);
        self.guide_dialog(ctx);
    }

    /// Added feature: *Manage Disk Space Usage*.
    ///
    /// The original's answer to "what is filling this disk?" is a Tools-menu
    /// action that launches Filelight (`manage_disk_space` → a submenu built by
    /// `FUN_001e8430` @0x1e8430, which looks for `org.kde.filelight` and offers
    /// to install it when it is missing). This window adds that missing piece
    /// itself: one walk of the folder, every child summed, largest first, with
    /// the biggest files below it listed so something can be done about them.
    fn space_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut sp) = self.space.take() else {
            return;
        };
        sp.poll();
        if sp.scanning {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        let mut win_open = true;
        let mut descend: Option<PathBuf> = None;
        let mut up = false;
        let mut rescan = false;
        let mut reveal: Option<PathBuf> = None;
        let mut pick: Option<PathBuf> = None;
        // The volume's own figures, from the same call the status bar uses.
        let volume = fsutil::free_space(&sp.root);
        let rows = sp.rows();
        let total = sp.shown_total();
        let biggest: Vec<(PathBuf, u64)> = sp
            .biggest_here()
            .into_iter()
            .take(14)
            .map(|b| (b.path.clone(), b.size))
            .collect();

        egui::Window::new("Manage Disk Space Usage")
            .open(&mut win_open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(720.0, 590.0))
            .min_width(640.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&egui::Style::default())
                    .fill(theme::c(43, 47, 51))
                    .inner_margin(Margin::same(14))
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0, theme::border())),
            )
            .show(ctx, |ui| {
                ui.set_min_width(680.0);
                // The volume line. The original's submenu offers three scopes —
                // "Disk Usage Statistics - current folder" (string 0x237538),
                // "- current device" (0x237560) and "- all devices" (0x237588);
                // this window walks the open folder, so it is the first one.
                if let Some((free, whole)) = volume {
                    if whole > 0 {
                        let pct = fsutil::used_percent(free, whole);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Disk Usage Statistics - current folder")
                                    .size(11.5)
                                    .color(theme::text_dim()),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} free out of {} ({}% used)",
                                        fsutil::format_size(free),
                                        fsutil::format_size(whole),
                                        pct
                                    ))
                                    .size(11.0)
                                    .color(theme::text_faint()),
                                );
                                let (bar, _) = ui.allocate_exact_size(
                                    Vec2::new(150.0, 6.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    CornerRadius::same(2),
                                    theme::c(48, 53, 58),
                                );
                                let filled = bar.width() * (pct as f32 / 100.0);
                                if filled > 0.5 {
                                    ui.painter().rect_filled(
                                        Rect::from_min_size(
                                            bar.min,
                                            Vec2::new(filled, bar.height()),
                                        ),
                                        CornerRadius::same(2),
                                        theme::accent(),
                                    );
                                }
                            });
                        });
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(sp.summary())
                        .size(12.0)
                        .color(theme::text()),
                );
                if sp.cancelled {
                    ui.label(
                        egui::RichText::new(
                            "The walk was stopped, so this is only what had been added up.",
                        )
                        .size(11.0)
                        .color(theme::text_faint()),
                    );
                }
                ui.add_space(6.0);
                // Where the list is looking, then the controls that move it.
                let here = sp
                    .show
                    .strip_prefix(&sp.root)
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let name = sp
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| sp.root.display().to_string());
                let label = if here.is_empty() {
                    name
                } else {
                    format!("{name} / {here}")
                };
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(theme::elide(
                            ui,
                            &label,
                            380.0,
                            FontId::proportional(12.0),
                        ))
                        .size(12.0)
                        .color(theme::text_dim()),
                    )
                    .on_hover_text(sp.show.display().to_string());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if sp.scanning {
                            ui.spinner();
                            if ui.small_button("Stop").clicked() {
                                sp.stop();
                            }
                        }
                        if sp.trail.is_empty() {
                            if ui.small_button("Scan again").clicked() {
                                rescan = true;
                            }
                        } else {
                            if ui.small_button("Top folder").clicked() {
                                sp.trail.clear();
                                sp.show = sp.root.clone();
                                sp.picked = None;
                            }
                            if ui.small_button("Up").clicked() {
                                up = true;
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                // Column headings, so the numbers have a place.
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
                    let p = ui.painter();
                    p.text(
                        Pos2::new(rect.left() + 30.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        "Name",
                        FontId::proportional(10.5),
                        theme::text_faint(),
                    );
                    p.text(
                        Pos2::new(rect.right() - 78.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        "Size",
                        FontId::proportional(10.5),
                        theme::text_faint(),
                    );
                    p.text(
                        Pos2::new(rect.right() - 8.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        "Share",
                        FontId::proportional(10.5),
                        theme::text_faint(),
                    );
                });
                egui::ScrollArea::vertical()
                    .max_height(236.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_min_width(660.0);
                        if rows.is_empty() {
                            ui.add_space(10.0);
                            if sp.scanning {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Adding up the folders…")
                                            .size(12.0)
                                            .color(theme::text_dim()),
                                    );
                                });
                            } else {
                                ui.label(
                                    egui::RichText::new("Nothing in this folder to add up.")
                                        .size(12.0)
                                        .color(theme::text_dim()),
                                );
                            }
                        }
                        for row in rows.iter() {
                            // The scroll bar takes the last few pixels of the
                            // list, so the figures stop short of the edge
                            // instead of being drawn under it.
                            let width = ui.available_width() - 14.0;
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(width, 26.0),
                                Sense::click(),
                            );
                            let share = space::share_percent(row.size, total);
                            let picked_here = sp.picked.as_deref() == Some(row.path.as_path());
                            if resp.hovered() || picked_here {
                                ui.painter().rect_filled(
                                    rect.shrink2(Vec2::new(2.0, 1.0)),
                                    CornerRadius::same(3),
                                    if picked_here { theme::row_sel_active() } else { theme::row_hover() },
                                );
                            } else if share > 0 {
                                // A thin band under the row, as wide as the
                                // entry's share: a bar chart that needs no
                                // second column and cannot be mistaken for the
                                // hovered row.
                                let band = Rect::from_min_size(
                                    Pos2::new(rect.left() + 2.0, rect.bottom() - 4.0),
                                    Vec2::new(
                                        (rect.width() - 4.0) * (share as f32 / 100.0),
                                        2.0,
                                    ),
                                );
                                ui.painter().rect_filled(
                                    band,
                                    CornerRadius::same(1),
                                    theme::accent().gamma_multiply(0.55),
                                );
                            }
                            let icon = Rect::from_center_size(
                                Pos2::new(rect.left() + 16.0, rect.center().y),
                                Vec2::splat(15.0),
                            );
                            theme::draw_kind_icon(ui.painter(), icon, row.kind, false);
                            let name = theme::elide(
                                ui,
                                &row.name,
                                rect.width() - 120.0,
                                FontId::proportional(12.5),
                            );
                            ui.painter().text(
                                Pos2::new(rect.left() + 30.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                name,
                                FontId::proportional(12.5),
                                theme::text(),
                            );
                            ui.painter().text(
                                Pos2::new(rect.right() - 78.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                fsutil::format_size(row.size),
                                FontId::monospace(11.0),
                                theme::text_dim(),
                            );
                            ui.painter().text(
                                Pos2::new(rect.right() - 8.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                format!("{share}%"),
                                FontId::monospace(11.0),
                                theme::text_faint(),
                            );
                            if resp.clicked() {
                                pick = Some(row.path.clone());
                            }
                            if resp.double_clicked() && row.is_dir {
                                descend = Some(row.path.clone());
                            }
                            resp.on_hover_text(format!(
                                "{}\n{}",
                                row.path.display(),
                                fsutil::format_size(row.size)
                            ));
                        }
                    });
                // What the list is looking at, and the ways out of the window.
                if let Some(picked) = sp.picked.clone() {
                    let is_dir = picked.is_dir();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(theme::elide(
                                ui,
                                &picked.display().to_string(),
                                360.0,
                                FontId::proportional(11.0),
                            ))
                            .size(11.0)
                            .color(theme::text_faint()),
                        )
                        .on_hover_text(picked.display().to_string());
                        if is_dir {
                            if ui.small_button("Walk into").clicked() {
                                descend = Some(picked.clone());
                            }
                        } else if ui.small_button("Show in folder").clicked() {
                            reveal = Some(picked.clone());
                        }
                    });
                } else {
                    ui.add_space(4.0);
                }
                ui.separator();
                // The shortlist. "Free Up Disk Space" is the original's own row
                // name from the same disk-space menu (string 0x22ec97).
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Free Up Disk Space")
                            .size(11.5)
                            .strong()
                            .color(theme::text()),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "the largest files below this folder ({} kept)",
                            space::BIGGEST_KEPT
                        ))
                        .size(10.5)
                        .color(theme::text_faint()),
                    );
                });
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .id_salt("space-biggest")
                    .max_height(150.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_min_width(660.0);
                        if biggest.is_empty() {
                            ui.label(
                                egui::RichText::new(if sp.scanning {
                                    "Still walking the folders…"
                                } else {
                                    "No files below this folder."
                                })
                                .size(11.5)
                                .color(theme::text_dim()),
                            );
                        }
                        for (path, size) in biggest.iter() {
                            let short = path
                                .strip_prefix(&sp.root)
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| path.display().to_string());
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(fsutil::format_size(*size))
                                        .monospace()
                                        .size(10.5)
                                        .color(theme::text_dim()),
                                );
                                let width = (ui.available_width() - 46.0).max(80.0);
                                ui.label(
                                    egui::RichText::new(theme::elide(
                                        ui,
                                        &short,
                                        width,
                                        FontId::proportional(11.0),
                                    ))
                                    .size(11.0)
                                    .color(theme::text()),
                                )
                                .on_hover_text(path.display().to_string());
                                if ui.small_button("Show").clicked() {
                                    reveal = Some(path.clone());
                                }
                            });
                        }
                    });
            });

        // A window closed by its own cross-button stops the walk it started:
        // there is nobody left to show the total to.
        if !win_open {
            sp.stop();
            return;
        }
        if let Some(path) = pick {
            sp.picked = Some(path);
        }
        if let Some(path) = descend {
            sp.picked = None;
            sp.descend(path);
        }
        if up {
            sp.up();
            sp.picked = None;
        }
        if rescan {
            self.space = Some(space::Space::start(sp.root.clone()));
        } else {
            self.space = Some(sp);
        }
        if let Some(path) = reveal {
            self.reqs.push(Request::Reveal(path));
        }
    }

    /// Added feature: *Compress to ZIP*.
    ///
    /// Nothing in `/usr/bin/dolphin` compresses anything — its right-click
    /// *Compress* submenu is installed by Ark's file-item-action plugin, and the
    /// program itself carries only the settings row `Browse compressed files as
    /// folders`. This window does the job in place: one name, one button, and
    /// the archive lands in the folder the items came from.
    fn compress_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.compress.take() else {
            return;
        };
        if c.running() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        c.poll_total();
        if c.total.is_none() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        // The archive program's answer, once it has one.
        let mut finished: Option<PathBuf> = None;
        if let Some(answer) = c.poll() {
            if answer.is_ok() {
                finished = c.last_dest.clone();
            }
        }
        if let Some(dest) = finished {
            if let Some(msg) = c.done_message() {
                self.toast(&msg);
            }
            // Show the new archive when it landed in the folder on show; when
            // the folder itself was compressed the archive sits beside it, and
            // jumping the person out of the folder they were reading would be
            // rude — the toast is enough there.
            let here = self.pane().current_dir();
            if dest.parent() == Some(here.as_path()) {
                self.reqs.push(Request::Reveal(dest));
            }
            return;
        }
        let total = c.total;
        let count = c.paths.len();
        let size_line = match total {
            Some(bytes) => fsutil::format_size(bytes),
            None => "adding up…".to_string(),
        };
        let mut win_open = true;
        let mut go = false;
        let mut close = false;
        window_style("Compress to ZIP")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(500.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} item{} ({})",
                        count,
                        if count == 1 { "" } else { "s" },
                        size_line
                    ))
                    .size(12.0)
                    .color(theme::text()),
                );
                ui.add_space(4.0);
                for p in c.paths.iter().take(6) {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string());
                    ui.label(
                        egui::RichText::new(theme::elide(
                            ui,
                            &name,
                            440.0,
                            FontId::proportional(11.5),
                        ))
                        .size(11.5)
                        .color(theme::text_dim()),
                    );
                }
                if count > 6 {
                    ui.label(
                        egui::RichText::new(format!("and {} more", count - 6))
                            .size(11.0)
                            .color(theme::text_faint()),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Name")
                            .size(11.5)
                            .color(theme::text_dim()),
                    );
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut c.name)
                            .desired_width(320.0)
                            .font(FontId::proportional(12.5)),
                    );
                    if c.focus {
                        edit.request_focus();
                        c.focus = false;
                    }
                });
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Will be written to {}",
                        theme::elide(
                            ui,
                            &c.dir.display().to_string(),
                            (ui.available_width() - 24.0).max(180.0),
                            FontId::proportional(10.5)
                        )
                    ))
                    .size(10.5)
                    .color(theme::text_faint()),
                )
                .on_hover_text(c.dir.display().to_string());
                if let Some(err) = c.error.clone() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(err)
                            .size(11.0)
                            .color(theme::c(224, 122, 95)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if c.running() {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Compressing…")
                                .size(11.5)
                                .color(theme::text_dim()),
                        );
                    } else {
                        if ui.button("Compress").clicked() {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if go {
            c.begin();
        }
        if close || !win_open {
            // A walk already handed to `zip` is left to finish: stopping it
            // halfway would leave a broken archive behind.
            if c.running() {
                self.compress = Some(c);
            }
            return;
        }
        self.compress = Some(c);
    }

    /// Added feature: *Extract*.
    ///
    /// The original hands an archive to Ark as well, so this window is an
    /// addition: it unpacks a ZIP with `unzip` and a tar archive with `tar` into
    /// a folder named after the file, or into the folder on show.
    fn extract_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut x) = self.extract.take() else {
            return;
        };
        if x.running() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        let mut finished: Option<PathBuf> = None;
        if let Some(answer) = x.poll() {
            if answer.is_ok() {
                finished = x.last_dest.clone();
            }
        }
        if let Some(dest) = finished {
            if let Some(msg) = x.done_message() {
                self.toast(&msg);
            }
            if x.own_folder {
                self.reqs.push(Request::Reveal(dest));
            } else {
                self.reqs.push(Request::Navigate(dest));
            }
            return;
        }
        let name = x
            .archive
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let size = std::fs::metadata(&x.archive)
            .map(|m| m.len())
            .unwrap_or(0);
        let dest = x.dest();
        let mut win_open = true;
        let mut go = false;
        let mut close = false;
        window_style("Extract Archive")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  {}  ·  {}",
                        theme::elide(ui, &name, 300.0, FontId::proportional(12.0)),
                        x.kind.label(),
                        fsutil::format_size(size)
                    ))
                    .size(12.0)
                    .color(theme::text()),
                );
                ui.add_space(8.0);
                ui.checkbox(&mut x.own_folder, "Into a folder of its own");
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Will be unpacked into {}",
                        theme::elide(
                            ui,
                            &dest.display().to_string(),
                            (ui.available_width() - 24.0).max(180.0),
                            FontId::proportional(10.5)
                        )
                    ))
                    .size(10.5)
                    .color(theme::text_faint()),
                )
                .on_hover_text(dest.display().to_string());
                if let Some(err) = x.error.clone() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(err)
                            .size(11.0)
                            .color(theme::c(224, 122, 95)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if x.running() {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Unpacking…")
                                .size(11.5)
                                .color(theme::text_dim()),
                        );
                    } else {
                        if ui.button("Extract").clicked() {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if go {
            x.begin();
        }
        if close || !win_open {
            if x.running() {
                self.extract = Some(x);
            }
            return;
        }
        self.extract = Some(x);
    }

    /// Added feature: *Copy To…* / *Move To…*.
    ///
    /// The original installs these two rows from KIO's `KFileCopyToMenu`, which
    /// opens a folder-picking dialog; the only trace of them in
    /// `/usr/bin/dolphin` is the settings page that turns them on, titled
    /// *'Copy To' and 'Move To' commands* (0x236a20). This window is the same
    /// idea built small: the folder is typed or picked from the list, the count
    /// of marked items is shown, and one button does the work. Nothing is
    /// overwritten — a name already in the folder gets `(1)` after it.
    fn copy_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.copy_chooser.take() else {
            return;
        };
        let mut win_open = true;
        let mut go = false;
        let mut close = false;
        let title = match c.mode {
            crate::copymove::Mode::Copy => "Copy To Folder",
            crate::copymove::Mode::Move => "Move To Folder",
        };
        let marked = c.paths.len();
        let target = c.target();
        window_style(title)
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(500.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} item{} marked",
                        marked,
                        if marked == 1 { "" } else { "s" }
                    ))
                    .size(11.5)
                    .color(theme::text_dim()),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Folder to go to")
                        .size(10.5)
                        .color(theme::text_faint()),
                );
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut c.text)
                        .desired_width(f32::INFINITY)
                        .hint_text("A folder, e.g. /home/you/Pictures"),
                );
                if c.focus {
                    resp.request_focus();
                    c.focus = false;
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    let places = c.places.clone();
                    for (label, path) in places {
                        let picked = target.as_ref() == Some(&path);
                        if ui.selectable_label(picked, label).on_hover_text(path.display().to_string()).clicked() {
                            c.text = path.display().to_string();
                            c.error = None;
                        }
                    }
                });
                ui.add_space(8.0);
                // Say what will happen, in the folder's own words: how many
                // items move, and the fact that nothing local is replaced.
                match &target {
                    Some(dest) => {
                        let plan = crate::copymove::plan(dest, &c.paths);
                        let collides = plan
                            .steps
                            .iter()
                            .filter(|s| {
                                dest.join(
                                    s.from
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                )
                                .exists()
                            })
                            .count();
                        let mut line = format!("{} item{} will go to {}", plan.steps.len(), if plan.steps.len() == 1 { "" } else { "s" }, theme::elide(ui, &dest.display().to_string(), (ui.available_width() - 20.0).max(180.0), FontId::proportional(10.5)));
                        if collides > 0 {
                            line.push_str(&format!(
                                " — {} name{} already there, so the new cop{} get{} a number instead of replacing anything",
                                collides,
                                if collides == 1 { "" } else { "s" },
                                if collides == 1 { "y" } else { "ies" },
                                if collides == 1 { "s" } else { "" }
                            ));
                        }
                        ui.label(egui::RichText::new(line).size(10.5).color(theme::text_faint()));
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("That folder does not exist yet.")
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                    }
                }
                if let Some(err) = c.error.clone() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(err)
                            .size(11.0)
                            .color(theme::c(224, 122, 95)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(c.mode.chooser_label().trim_end_matches('…')).clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if go {
            match c.check() {
                Ok(dest) => {
                    let mode = c.mode;
                    let paths = c.paths.clone();
                    self.reqs.push(Request::CopyToChosen { mode, paths, dest });
                    return;
                }
                Err(why) => {
                    c.error = Some(why);
                    self.copy_chooser = Some(c);
                    return;
                }
            }
        }
        if close || !win_open {
            return;
        }
        self.copy_chooser = Some(c);
    }

    /// Added feature, asked for this round: *Resize Image…*.
    ///
    /// The window asks for one number — a size in **megapixels** — and shows, as
    /// it is typed, exactly what that number does to the picture: the width and
    /// height it comes out at and the shape the two keep. Dolphin itself has no
    /// resize action (its picture rows stop at *Open With* and the plugins), so
    /// the wording, the defaults and the file naming here are this build's own;
    /// the unit is the one the original's size caption uses (`MP` =
    /// width × height ÷ 1 000 000).
    fn resize_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.resize.take() else {
            return;
        };
        if d.running() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        // The worker's answer. Once every picture has been through it, the
        // window closes and the toast says what happened.
        if let Some(outcome) = d.poll() {
            let msg = outcome.message(d.last_mp);
            if !outcome.written.is_empty() {
                let here = self.pane().current_dir();
                let first = outcome.written[0].clone();
                if first.parent() == Some(here.as_path()) {
                    self.reqs.push(Request::Reveal(first));
                }
            }
            self.toast(&msg);
            return;
        }

        let count = d.items.len();
        let ready = d.ready().count();
        let first = d.items.first().map(|i| i.name()).unwrap_or_default();
        let preview = d.preview();
        let mp_error = d.mp().err();
        let any_jpeg = d.any_jpeg();
        let replace = d.replace;
        let mut go = false;
        let mut close = false;
        let mut win_open = true;
        let title = if count > 1 { "Resize Images" } else { "Resize Image" };
        window_style(title)
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(540.0);

                // What is being resized: one name, or a count and the first name.
                let heading = if count == 1 {
                    first.clone()
                } else {
                    format!("{} pictures, starting with {}", count, first)
                };
                ui.label(
                    egui::RichText::new(theme::elide(ui, &heading, 500.0, FontId::proportional(12.5)))
                        .size(12.5)
                        .color(theme::text()),
                )
                .on_hover_text(first.clone());
                if count > 1 {
                    if let Ok(mp) = d.mp() {
                        ui.label(
                            egui::RichText::new(format!(
                                "Each of the {count} pictures is scaled to {} MP on its own, keeping its own shape — the numbers below are the first one's.",
                                crate::imageresize::format_mp(mp)
                            ))
                            .size(10.5)
                            .color(theme::text_faint()),
                        );
                    }
                }

                if let Some((from, to)) = preview {
                    // The whole point of the window: now on the left, after on
                    // the right, with the shape named in between so it is clear
                    // that the ratio is what is being kept.
                    ui.add_space(8.0);
                    let now = format!("{} × {}", from.0, from.1);
                    let after = format!("{} × {}", to.0, to.1);
                    let shape = crate::gallery::ratio_label(from.0, from.1);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Now")
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                        ui.label(egui::RichText::new(now).size(12.0).color(theme::text_dim()));
                        ui.label(
                            egui::RichText::new(format!("  ·  {:.2} MP", crate::imageresize::megapixels(from.0, from.1)))
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("to")
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(after).size(12.0).color(theme::text()));
                        ui.label(
                            egui::RichText::new(format!(
                                "  ·  {:.2} MP",
                                crate::imageresize::megapixels(to.0, to.1)
                            ))
                            .size(10.5)
                            .color(theme::text_faint()),
                        );
                        ui.label(
                            egui::RichText::new(format!("  ·  {shape} kept"))
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                    });
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Megapixels").size(11.5).color(theme::text_dim()));
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut d.mp_text)
                            .desired_width(74.0)
                            .font(FontId::proportional(13.0)),
                    );
                    if d.focus {
                        edit.request_focus();
                        d.focus = false;
                    }
                    ui.label(egui::RichText::new("MP").size(11.5).color(theme::text_faint()));
                    ui.add_space(10.0);
                    // The sizes people usually want, one click each.
                    for preset in [1.0_f64, 2.0, 4.0, 8.0, 12.0, 24.0] {
                        let label = crate::imageresize::format_mp(preset);
                        if ui
                            .small_button(&label)
                            .on_hover_text(format!("{label} megapixels"))
                            .clicked()
                        {
                            d.mp_text = label.clone();
                        }
                    }
                });
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(
                        "Only the number of pixels changes: the width and the height are scaled by the same factor, so the picture keeps its shape.",
                    )
                    .size(10.5)
                    .color(theme::text_faint()),
                );

                if let Some(err) = mp_error.clone() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(err)
                            .size(11.0)
                            .color(theme::c(224, 122, 95)),
                    );
                }
                if let Some((from, to)) = preview {
                    if crate::imageresize::megapixels(to.0, to.1)
                        > crate::imageresize::megapixels(from.0, from.1) * 1.001
                    {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "That is larger than the picture itself — the extra pixels are made up by the scaler, not recovered.",
                            )
                            .size(10.5)
                            .color(theme::c(226, 192, 122)),
                        );
                    }
                }
                // Anything in the selection that cannot go through is named, so
                // a folder or a raw file in the batch is not silently dropped.
                let skipped: Vec<&crate::imageresize::Item> = d
                    .items
                    .iter()
                    .filter(|i| i.note.is_some())
                    .collect();
                if !skipped.is_empty() {
                    ui.add_space(6.0);
                    for item in skipped.iter().take(3) {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} — {}",
                                theme::elide(ui, &item.name(), 240.0, FontId::proportional(10.5)),
                                item.note.clone().unwrap_or_default()
                            ))
                            .size(10.5)
                            .color(theme::text_faint()),
                        );
                    }
                    if skipped.len() > 3 {
                        ui.label(
                            egui::RichText::new(format!("and {} more left as they are", skipped.len() - 3))
                                .size(10.5)
                                .color(theme::text_faint()),
                        );
                    }
                }

                ui.add_space(10.0);
                ui.checkbox(&mut d.replace, "Replace the original")
                    .on_hover_text(
                        "Off: a copy is written beside the picture, named after its size. On: the picture itself is overwritten.",
                    );
                if replace {
                    ui.label(
                        egui::RichText::new("The original is overwritten; there is no separate copy.")
                            .size(10.5)
                            .color(theme::c(226, 192, 122)),
                    );
                }
                // The name the copy gets, so the result is predictable before
                // the button is pressed.
                if let Some(item) = d.items.first() {
                    if !replace && item.from.is_some() {
                        let mp = d.mp().unwrap_or(crate::imageresize::DEFAULT_MP);
                        let out = crate::imageresize::output_path(&item.src, mp, false);
                        let name = out
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Will be saved as {} beside the original",
                                theme::elide(ui, &name, 360.0, FontId::proportional(10.5))
                            ))
                            .size(10.5)
                            .color(theme::text_faint()),
                        )
                        .on_hover_text(out.display().to_string());
                    }
                }
                if any_jpeg {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("JPEG quality").size(11.5).color(theme::text_dim()));
                        ui.spacing_mut().slider_width = 170.0;
                        ui.add(
                            egui::Slider::new(&mut d.quality, 40..=100)
                                .suffix("%")
                                .show_value(true),
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "Only JPEG files use this; PNG, WebP, BMP and TIFF are written losslessly.",
                        )
                        .size(10.5)
                        .color(theme::text_faint()),
                    );
                }

                if let Some(err) = d.error.clone() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(err)
                            .size(11.0)
                            .color(theme::c(224, 122, 95)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if d.running() {
                        ui.spinner();
                        let work = d.work.as_ref();
                        let (done, total) = work.map(|w| (w.done, w.total)).unwrap_or((0, 0));
                        ui.label(
                            egui::RichText::new(format!("Resizing {done} of {total}…"))
                                .size(11.5)
                                .color(theme::text_dim()),
                        );
                    } else {
                        ui.add_enabled_ui(ready > 0 && mp_error.is_none(), |ui| {
                            // Off the replace box the button writes a copy, so it
                            // says exactly that; on, it overwrites the picture.
                            let label = if replace { "Resize" } else { "Duplicate" };
                            if ui.button(label).clicked() {
                                go = true;
                            }
                        });
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if go {
            d.begin();
        }
        if close || !win_open {
            // A run already handed to the worker is left to finish: stopping it
            // halfway would leave a half-written copy behind.
            if d.running() {
                self.resize = Some(d);
            }
            return;
        }
        self.resize = Some(d);
    }

    /// Added feature: "Other Application…" — the chooser the original opens
    /// from the bottom of its "Open With" submenu (`&Other Application…`
    /// @0x11c387 in libKF6KIOWidgets.so.6). Every program the desktop knows is
    /// listed, narrowed as the person types, with the original's own tick box
    /// underneath: `&Remember application association for all files of type
    /// "%1" (%2)` @0x120210, where %1 is the extension and %2 the type.
    fn open_with_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ow) = self.open_with.take() else {
            return;
        };
        let mut win_open = true;
        let mut close = false;
        let mut run: Option<String> = None;
        let all = crate::apps::chooser(&ow.mime);
        let needle = ow.filter.to_lowercase();
        let matching: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, a)| needle.is_empty() || a.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        let chosen = ow.chosen.min(matching.len().saturating_sub(1));
        let what = match ow.paths.len() {
            0 => String::new(),
            1 => ow
                .paths
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            n => format!("{} items", n),
        };
        window_style("Open With")
            .open(&mut win_open)
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.label(egui::RichText::new(what.clone()).size(12.5).color(theme::text_dim()));
                ui.add_space(6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut ow.filter)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type to narrow the list"),
                );
                if !ow.focus {
                    search.request_focus();
                    ow.focus = true;
                }
                ui.add_space(6.0);
                let height = 18.0 * matching.len().min(11) as f32 + 8.0;
                egui::ScrollArea::vertical()
                    .max_height(height.max(60.0))
                    .show(ui, |ui| {
                        for (row, &i) in matching.iter().enumerate() {
                            let app = &all[i];
                            let picked = row == chosen;
                            let label = egui::RichText::new(format!("{}  {}", app.name, ""))
                                .size(12.5)
                                .color(if picked { theme::on_band() } else { theme::text_dim() });
                            let resp = ui.add(
                                egui::Button::new(label)
                                    .fill(if picked {
                                        theme::c(61, 130, 180)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .min_size(Vec2::new(ui.available_width(), 18.0)),
                            );
                            if resp.clicked() {
                                run = Some(app.id.clone());
                            }
                            if resp.double_clicked() {
                                run = Some(app.id.clone());
                            }
                        }
                        if matching.is_empty() {
                            ui.label(
                                egui::RichText::new("No program on this computer has that name.")
                                    .size(12.0)
                                    .color(theme::text_faint()),
                            );
                        }
                    });
                ui.add_space(8.0);
                let ext = ow
                    .paths
                    .first()
                    .and_then(|p| p.extension())
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let remember_label = crate::apps::remember_label(&ext, &ow.mime);
                ui.checkbox(&mut ow.remember, remember_label);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enabled = !matching.is_empty();
                        if ui
                            .add_enabled(enabled, egui::Button::new("Open"))
                            .clicked()
                        {
                            run = Some(all[matching[chosen]].id.clone());
                        }
                    });
                });
                // The arrow keys walk the list, Enter opens, Escape closes —
                // the way a chooser is used without the mouse.
                if search.has_focus() || ui.memory(|m| m.focused().is_some()) {
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) && !matching.is_empty() {
                        ow.chosen = (chosen + 1).min(matching.len() - 1);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        ow.chosen = chosen.saturating_sub(1);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !matching.is_empty() {
                        run = Some(all[matching[chosen]].id.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                }
            });
        if let Some(id) = run {
            self.reqs.push(Request::OpenWith {
                id,
                paths: ow.paths.clone(),
                remember: if ow.remember {
                    Some(ow.mime.clone())
                } else {
                    None
                },
            });
            return;
        }
        if close || !win_open {
            return;
        }
        ow.chosen = ow.chosen.min(matching.len().saturating_sub(1));
        self.open_with = Some(ow);
    }

    /// Added: the plain-language answer to "what do these three buttons do?".
    /// One row per button, each naming the one thing it owns — a place, a find,
    /// a setting — because that is the confusion a row of unlabeled glyphs
    /// creates.
    fn guide_dialog(&mut self, ctx: &egui::Context) {
        if !self.guide_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        let rows: [(&str, &str, &str); 2] = [
            (
                "search",
                "Find",
                "Narrows what you see to the names that match. “This folder” \
                 looks at the list in front of you while you type; “In subfolders” \
                 also looks inside every folder below.",
            ),
            (
                "menu",
                "Menu",
                "The app's own switches and extras — sorting, previews, hidden \
                 files, duplicates, bulk rename. Nothing in this list changes your \
                 files on its own.",
            ),
        ];
        window_style("What the Two Buttons Do")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.label(
                    egui::RichText::new(
                        "They sit together at the right end of the toolbar. Each one owns a \
                         different question.",
                    )
                    .size(11.5)
                    .color(theme::text_dim()),
                );
                ui.add_space(10.0);
                for (glyph, name, text) in rows {
                    ui.horizontal_top(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(4),
                            theme::c(52, 58, 64),
                        );
                        theme::draw_glyph(ui.painter(), rect, glyph, theme::text());
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(360.0);
                            ui.label(egui::RichText::new(name).size(13.0).color(theme::text()));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text).size(11.5).color(theme::text_dim()),
                                )
                                .wrap(),
                            );
                        });
                    });
                    ui.add_space(10.0);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Right-click a file or folder for its own menu.")
                            .size(10.5)
                            .color(theme::text_faint()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("Close").min_size(Vec2::new(84.0, 24.0)))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
            });
        if !open || close {
            self.guide_open = false;
        }
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
                ui.label(egui::RichText::new(&confirm.body).size(13.0).color(theme::text()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(90.0, 26.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // The continue buttons carry the original's own words:
                        // "Move to Trash", "Delete Permanently" and "Empty Trash"
                        // (`KIO::JobUiDelegate::askDeleteConfirmation` @0x1a8a20,
                        // kio 6.31.0; strings 0x21e94c and the two above).
                        let (label, color) = match &confirm.action {
                            ConfirmAction::Trash(_) => {
                                ("Move to Trash", theme::c(61, 130, 180))
                            }
                            ConfirmAction::Restore(_) => {
                                ("Restore to Former Location", theme::c(61, 130, 180))
                            }
                            ConfirmAction::EmptyTrash => {
                                (crate::EMPTY_TRASH_LABEL, theme::c(170, 68, 62))
                            }
                            ConfirmAction::Delete(_) => {
                                (crate::DELETE_PERMANENT_LABEL, theme::c(170, 68, 62))
                            }
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label).color(theme::on_band()),
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
            // The switch the person turned on only puts a question first; the
            // deed and its toast are the same either way.
            ConfirmAction::Trash(paths) => self.trash_now(paths),
            ConfirmAction::Delete(paths) => self.delete_now(paths),
            ConfirmAction::EmptyTrash => self.empty_trash_now(),
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
                            tint.map(|c| theme::c(c[0], c[1], c[2])),
                        );
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&entry.name).size(14.0).strong());
                            ui.label(
                                egui::RichText::new(entry.type_label())
                                    .size(12.0)
                                    .color(theme::text_dim()),
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
                            .color(theme::text_dim()),
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
                                    .color(if props.checksum.is_some() { theme::text() } else { theme::text_faint() }),
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
                // Same recursion limit the original's size job gets
                // (`ContentDisplaySettings::RecursiveDirectorySizeLimit`).
                let depth = self
                    .config
                    .size_limit
                    .clamp(crate::search::SIZE_LIMIT_MIN, crate::search::SIZE_LIMIT_MAX);
                std::thread::spawn(move || {
                    let size = crate::search::dir_size_limited(&p, Some(depth));
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
                        .color(theme::text_dim()),
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
                        .color(theme::text_faint()),
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
                                        egui::RichText::new(old).size(11.5).color(theme::text_dim()),
                                    )
                                    .truncate(),
                                );
                                ui.label(egui::RichText::new("→").size(11.5).color(theme::text_faint()));
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(new).size(11.5).color(theme::text()),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                    });
                if let Some(err) = &bulk.error {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(err).size(11.5).color(theme::c(220, 120, 110)));
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

    /// Added feature: *Find in Files*.
    ///
    /// The original's find bar names two content modes — `File names and
    /// contents` (string 0x22e0db in `/usr/bin/dolphin`) and `File contents`
    /// (0x22e10c) — and then hands the reading to KIO's search helper. This
    /// window does the reading itself, so the two modes answer here even when
    /// no helper is installed: one row per file, with the line the phrase was
    /// written on.
    fn findtext_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ft) = self.findtext.take() else {
            return;
        };
        ft.poll();
        if ft.scanning {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mut win_open = true;
        let mut close = false;
        let mut start: Option<findtext::Options> = None;
        let mut stop = false;
        let mut reveal: Option<PathBuf> = None;
        let mut open_file: Option<PathBuf> = None;
        let root = ft.root.clone();
        let summary = ft.summary();
        let scanning = ft.scanning;
        let hits = ft.hits.clone();
        let phrase_empty = ft.opts.phrase.trim().is_empty();

        window_style("Find in Files")
            .open(&mut win_open)
            .default_size(Vec2::new(700.0, 540.0))
            .show(ctx, |ui| {
                ui.set_min_width(660.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Folder:").size(11.0).color(theme::text_dim()));
                    ui.label(
                        egui::RichText::new(root.display().to_string())
                            .size(11.0)
                            .color(theme::text_dim()),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Search for:").size(12.0));
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut ft.opts.phrase)
                            .hint_text("a word or a sentence")
                            .desired_width(380.0),
                    );
                    if ft.focus {
                        field.request_focus();
                        ft.focus = false;
                    }
                    // Enter in the field starts the search, the same as pressing
                    // the button: the field is the reason the window is open.
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if !ft.opts.phrase.trim().is_empty() {
                            start = Some(ft.opts.clone());
                        }
                    }
                });
                ui.add_space(6.0);
                // The three modes, in the original's own wording for the two it
                // names: a plain name search, names and contents together, or
                // contents alone.
                ui.horizontal(|ui| {
                    for mode in [
                        findtext::Mode::Names,
                        findtext::Mode::NamesAndContents,
                        findtext::Mode::Contents,
                    ] {
                        if ui
                            .selectable_label(ft.opts.mode == mode, mode.label())
                            .clicked()
                        {
                            ft.opts.mode = mode;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut ft.opts.case_sensitive, "Case sensitive");
                    ui.add_space(10.0);
                    ui.checkbox(&mut ft.opts.include_binary, "Include binary files");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if scanning {
                            if ui.button("Stop").clicked() {
                                stop = true;
                            }
                        } else if ui
                            .add_sized([72.0, 22.0], egui::Button::new("Find"))
                            .clicked()
                        {
                            start = Some(ft.opts.clone());
                        }
                    });
                });
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(summary)
                        .size(11.0)
                        .color(theme::text_dim()),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if hits.is_empty() {
                            let line = if phrase_empty {
                                "Type a phrase, then press Enter."
                            } else if scanning {
                                "Looking inside the files…"
                            } else {
                                "No file in this folder holds that phrase."
                            };
                            ui.label(egui::RichText::new(line).size(12.0).color(theme::text_dim()));
                        }
                        for hit in hits.iter() {
                            egui::Frame::new()
                                .fill(theme::c(37, 41, 45))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(Margin::symmetric(8, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(hit.file_name())
                                                .size(12.5)
                                                .color(theme::text())
                                                .strong(),
                                        );
                                        if hit.matches > 1 {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} matches",
                                                    hit.matches
                                                ))
                                                .size(10.0)
                                                .color(theme::text_faint()),
                                            );
                                        }
                                        ui.label(
                                            egui::RichText::new(format!("— {}", hit.folder()))
                                                .size(10.0)
                                                .color(theme::text_faint()),
                                        );
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui.small_button("Open").clicked() {
                                                    open_file = Some(hit.path.clone());
                                                }
                                                if ui.small_button("Show").clicked() {
                                                    reveal = Some(hit.path.clone());
                                                }
                                            },
                                        );
                                    });
                                    if hit.line_no > 0 {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}: {}",
                                                hit.line_no, hit.line
                                            ))
                                            .monospace()
                                            .size(10.5)
                                            .color(theme::text_dim()),
                                        );
                                    }
                                });
                            ui.add_space(4.0);
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });

        if stop {
            ft.stop();
        }
        if close {
            win_open = false;
        }
        if let Some(opts) = start {
            let mut fresh = findtext::FindText::start(root, opts);
            fresh.poll();
            self.findtext = Some(fresh);
        } else if !win_open {
            self.findtext = None;
        } else {
            self.findtext = Some(ft);
        }
        if let Some(path) = reveal {
            self.reqs.push(Request::Reveal(path));
        }
        if let Some(path) = open_file {
            self.reqs.push(Request::OpenPath(path));
        }
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
                            .color(theme::text_dim()),
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
                            .color(theme::text_dim()),
                        );
                    });
                } else if dups.scan.groups.is_empty() {
                    ui.label(
                        egui::RichText::new("No duplicates found.")
                            .size(13.0)
                            .color(theme::text_dim()),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} groups, {} could be freed",
                            dups.scan.groups.len(),
                            fsutil::format_size(dups.scan.wasted())
                        ))
                        .size(12.0)
                        .color(theme::text()),
                    );
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (gi, group) in dups.scan.groups.iter().enumerate() {
                            egui::Frame::new()
                                .fill(theme::c(37, 41, 45))
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
                                                .color(theme::text_faint()),
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
                                        let color = if i == 0 { theme::text() } else { theme::text_dim() };
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
                    .fill(theme::c(26, 29, 32))
                    .inner_margin(Margin::same(12))
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0, theme::border())),
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
                        tint.map(|c| theme::c(c[0], c[1], c[2])),
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
                            .color(theme::text_faint()),
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
                                .color(theme::text_dim()),
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
                            ui.label(egui::RichText::new("Loading preview…").color(theme::text_dim()));
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
                                .color(theme::text_dim()),
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
                    .color(theme::text_dim()),
            );
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Name").size(11.0).color(theme::text_dim()));
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
            ui.label(egui::RichText::new("Folder").size(11.0).color(theme::text_dim()));
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
                        .color(theme::c(214, 120, 112)),
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
                                    .color(theme::on_band()),
                            )
                            .fill(theme::c(61, 130, 180))
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
                        .color(theme::text_dim()),
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
                egui::Stroke::new(1.0, theme::text_dim()),
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
                        .color(theme::text_dim()),
                );
            },
        );
        ui.add_space(6.0);
        let width = ui.available_width().min(140.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(3),
            theme::c(28, 32, 35),
            egui::Stroke::new(1.0, theme::border()),
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
                        .color(theme::text_dim()),
                );
            },
        );
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(egui::RichText::new(value).size(11.5).color(theme::text()))
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
