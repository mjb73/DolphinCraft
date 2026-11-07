//! Added feature: the picture gallery, its full-window viewer, the slideshow and
//! the picture comparison — all adopted from the person's image browser
//! (`~/personal-files/python-projects/image-browser-styled`).
//!
//! The values here are that browser's own, read from its sources:
//!
//! | what | value | where it comes from |
//! | --- | --- | --- |
//! | thumbnail size slider | 50..400 px, 200 px at first | `templates/browser.html` (`#sizeSlider` min/max/value), `static/js/browser.js` `updateImageSize` |
//! | grid gap / cell padding | 12 px gap, 8 px cell padding, 10 px picture corner | `static/css/browser.css` `.images`, `.image`, `.image img` |
//! | dimension caption | `W×H:AR a.bc:MP d.ef` | `templates/browser.html`, the `show_dims` block |
//! | viewer zoom range | 0.1 .. 20 | `static/js/browser.js` `stepCompareZoom` |
//! | zoom buttons | ×1.25 / ÷1.25 | `compareZoomIn` / `compareZoomOut` |
//! | wheel zoom | ×1.1 in, ×0.9 out | `initCompareZoomWheel` |
//! | slideshow interval | 1 s first, 0.5 s steps | `#slide-interval` (`min="0.5" value="1" step="0.5"`) |
//! | slide fade | 300 ms | `startSlideshow` (`setTimeout(..., 300)`) |
//! | compare modes | side, split, overlay | `#compare-modal` toolbar |
//! | overlay opacity | 50 % first | `#compareOpacitySlider value="50"` |
//!
//! Nothing is drawn from the original browser's code: only these numbers and the
//! behaviour they describe were taken over.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, Sense, Stroke,
    Vec2,
};

use crate::fsutil::{self, Entry, Kind};
use crate::search::probe_image_size;
use crate::state::{Pane, Request};
use crate::theme::{self, *};
use crate::views::ViewCtx;

// ---------------------------------------------------------------------------
// The browser's numbers
// ---------------------------------------------------------------------------

/// Thumbnail edge range and first value of the browser's size slider.
pub const THUMB_MIN: f32 = 50.0;
pub const THUMB_MAX: f32 = 400.0;
pub const THUMB_DEFAULT: f32 = 200.0;
/// Gap between two cards and padding inside one, from the browser's CSS.
pub const THUMB_GAP: f32 = 12.0;
pub const CELL_PAD: f32 = 8.0;
pub const PICTURE_CORNER: u8 = 10;
/// Padding around the whole grid.
pub const GRID_PAD: f32 = 16.0;

/// Viewer zoom limits and steps.
pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 20.0;
pub const ZOOM_BUTTON_STEP: f32 = 1.25;
pub const ZOOM_WHEEL_IN: f32 = 1.1;
pub const ZOOM_WHEEL_OUT: f32 = 0.9;

/// Slideshow: first interval, smallest step, fade length.
pub const SLIDE_DEFAULT: f32 = 1.0;
pub const SLIDE_MIN: f32 = 0.5;
pub const SLIDE_STEP: f32 = 0.5;
pub const SLIDE_MAX: f32 = 30.0;
pub const FADE_MS: u64 = 300;

/// Compare overlay's first opacity, in percent.
pub const OPACITY_DEFAULT: u8 = 50;

pub fn clamp_thumb(px: f32) -> f32 {
    px.clamp(THUMB_MIN, THUMB_MAX)
}

pub fn clamp_zoom(z: f32) -> f32 {
    z.clamp(ZOOM_MIN, ZOOM_MAX)
}

/// One zoom step, clamped to the viewer's range.
pub fn zoom_step(z: f32, factor: f32) -> f32 {
    clamp_zoom(z * factor)
}

pub fn clamp_slide(secs: f32) -> f32 {
    secs.clamp(SLIDE_MIN, SLIDE_MAX)
}

/// The caption the browser prints under a thumbnail when "Show dimensions" is
/// ticked: `1920×1080:AR 1.78:MP 2.07`. Sizes the browser could not read are
/// reported in words, exactly as it does.
pub fn dims_caption(size: Option<(u32, u32)>) -> String {
    match size {
        Some((w, h)) if w > 0 && h > 0 => format!(
            "{}×{}:AR {:.2}:MP {:.2}",
            w,
            h,
            w as f32 / h as f32,
            (w as f64 * h as f64) / 1_000_000.0
        ),
        _ => "Dimensions unavailable".to_string(),
    }
}

/// "2.07 MP" on its own, for the information panel row.
pub fn megapixels(w: u32, h: u32) -> String {
    format!("{:.2} MP", (w as f64 * h as f64) / 1_000_000.0)
}

/// "16:9" when the ratio is one of the usual ones, otherwise "1.78:1".
pub fn ratio_label(w: u32, h: u32) -> String {
    if w == 0 || h == 0 {
        return "—".to_string();
    }
    let r = w as f32 / h as f32;
    for (a, b) in [
        (1.0f32, 1u32),
        (4.0, 3),
        (3.0, 2),
        (16.0, 9),
        (16.0, 10),
        (21.0, 9),
        (2.0, 1),
        (5.0, 4),
        (9.0, 16),
        (2.0, 3),
    ] {
        if (r - a / b as f32).abs() < 0.01 {
            return format!("{}:{}", a as u32, b);
        }
    }
    format!("{:.2}:1", r)
}

// ---------------------------------------------------------------------------
// Full-size pictures, loaded off the UI thread
// ---------------------------------------------------------------------------

/// Longest edge a picture is decoded at. A 100-megapixel panorama would need
/// half a gigabyte as raw RGBA, and no screen can show it; 4096 px is beyond a
/// 4K window at 100 % and still sharp when zoomed in.
pub const BIG_MAX_EDGE: u32 = 4096;

struct BigJob {
    path: PathBuf,
}

struct BigResult {
    path: PathBuf,
    pixels: Option<(usize, usize, Vec<u8>)>,
}

/// Textures of full-size pictures for the viewer, decoded on worker threads and
/// kept for the pictures nearest the one on screen.
pub struct BigCache {
    textures: HashMap<PathBuf, egui::TextureHandle>,
    order: Vec<PathBuf>,
    failed: Vec<PathBuf>,
    queue: Arc<Mutex<Vec<BigJob>>>,
    rx: Receiver<BigResult>,
    loading: Vec<PathBuf>,
}

impl BigCache {
    pub fn new() -> Self {
        let queue: Arc<Mutex<Vec<BigJob>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel::<BigResult>();
        for _ in 0..2 {
            let q = queue.clone();
            let tx: Sender<BigResult> = tx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let mut lock = q.lock().unwrap();
                    if lock.is_empty() {
                        None
                    } else {
                        Some(lock.remove(0))
                    }
                };
                let Some(job) = job else {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                };
                let pixels = match image::open(&job.path) {
                    Ok(img) => {
                        let (w, h) = (img.width(), img.height());
                        let img = if w.max(h) > BIG_MAX_EDGE {
                            let scale = BIG_MAX_EDGE as f32 / w.max(h) as f32;
                            img.resize(
                                (w as f32 * scale).round().max(1.0) as u32,
                                (h as f32 * scale).round().max(1.0) as u32,
                                image::imageops::FilterType::Triangle,
                            )
                        } else {
                            img
                        };
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        Some((w as usize, h as usize, rgba.into_raw()))
                    }
                    Err(_) => None,
                };
                let _ = tx.send(BigResult {
                    path: job.path,
                    pixels,
                });
            });
        }
        Self {
            textures: HashMap::new(),
            order: Vec::new(),
            failed: Vec::new(),
            queue,
            rx,
            loading: Vec::new(),
        }
    }

    /// The texture for a picture, asking for it to be decoded if it is not here
    /// yet.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        self.poll(ctx);
        if let Some(tex) = self.textures.get(path) {
            // Keep recently used pictures last in the order, so the one on
            // screen survives the trimming below.
            self.order.retain(|p| p != path);
            self.order.push(path.to_path_buf());
            return Some(tex.clone());
        }
        if !self.failed.iter().any(|p| p == path) && !self.loading.iter().any(|p| p == path) {
            self.loading.push(path.to_path_buf());
            self.queue.lock().unwrap().push(BigJob {
                path: path.to_path_buf(),
            });
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        None
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut keep_repainting = false;
        while let Ok(result) = self.rx.try_recv() {
            self.loading.retain(|p| p != &result.path);
            match result.pixels {
                Some((w, h, rgba)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture(
                        format!("picture:{}", result.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(result.path.clone(), tex);
                    self.order.retain(|p| p != &result.path);
                    self.order.push(result.path);
                }
                None => self.failed.push(result.path),
            }
            keep_repainting = true;
        }
        // Four pictures is enough for the one on screen and the two either side
        // of it, and keeps a folder of 40-megapixel photos from eating memory.
        while self.order.len() > 4 {
            let oldest = self.order.remove(0);
            self.textures.remove(&oldest);
        }
        if !self.loading.is_empty() || keep_repainting {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.textures.remove(path);
        self.order.retain(|p| p != path);
        self.failed.retain(|p| p != path);
    }
}

// ---------------------------------------------------------------------------
// Gallery cards
// ---------------------------------------------------------------------------

/// Height the two caption lines take under a thumbnail.
fn caption_height(names: bool, dims: bool) -> f32 {
    let mut h = 0.0;
    if names {
        h += 17.0;
    }
    if dims {
        h += 15.0;
    }
    h
}

/// One card's footprint. The picture always sits in a square of the chosen
/// size; the browser does the same (its images are `width: var(--thumb-size)`),
/// which is what makes a grid of mixed portrait and landscape shots line up.
pub fn card_size(thumb: f32, names: bool, dims: bool) -> Vec2 {
    Vec2::new(
        thumb + 2.0 * CELL_PAD,
        thumb + 2.0 * CELL_PAD + caption_height(names, dims),
    )
}

/// How many cards fit in a row, and the grid's total height.
pub fn grid_columns(avail: f32, card_w: f32) -> usize {
    let usable = (avail - 2.0 * GRID_PAD + THUMB_GAP).max(card_w);
    (usable / (card_w + THUMB_GAP)).floor().max(1.0) as usize
}

pub fn grid_rows(count: usize, columns: usize) -> usize {
    if columns == 0 {
        0
    } else {
        count.div_ceil(columns)
    }
}

impl Pane {
    /// The row of controls above the grid: the browser's size slider, its
    /// "Hide filenames" and "Show dimensions" switches, the picture count and
    /// the two things the gallery adds to the file manager — a slideshow and a
    /// comparison.
    fn gallery_bar(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        // A fixed-height row: `horizontal_centered` would stretch to the whole
        // view and squeeze the grid out of the window.
        let row = Vec2::new(ui.available_width(), 24.0);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                egui::RichText::new("Size:")
                    .size(11.5)
                    .color(TEXT_DIM),
            );
            let mut px = self.thumb_px;
            let slider = ui.add_sized(
                Vec2::new(132.0, 16.0),
                egui::Slider::new(&mut px, THUMB_MIN..=THUMB_MAX)
                    .show_value(false)
                    .step_by(1.0),
            );
            if slider.changed() {
                self.thumb_px = clamp_thumb(px);
                vctx.reqs.push(Request::GalleryChanged);
            }
            ui.label(
                egui::RichText::new(format!("{} px", self.thumb_px.round() as u32))
                    .size(11.0)
                    .color(TEXT_FAINT),
            );

            let mut names = self.gallery_names;
            if ui
                .add(egui::Checkbox::new(&mut names, egui::RichText::new("Names").size(11.5)))
                .on_hover_text("Show the file name under each picture; untick to hide it")
                .changed()
            {
                self.gallery_names = names;
                vctx.reqs.push(Request::GalleryChanged);
            }
            let mut dims = self.gallery_dims;
            if ui
                .add(egui::Checkbox::new(
                    &mut dims,
                    egui::RichText::new("Dimensions").size(11.5),
                ))
                .on_hover_text("Show width × height, aspect ratio and megapixels")
                .changed()
            {
                self.gallery_dims = dims;
                vctx.reqs.push(Request::GalleryChanged);
            }
            let mut round = self.gallery_round;
            if ui
                .add(egui::Checkbox::new(
                    &mut round,
                    egui::RichText::new("Rounded").size(11.5),
                ))
                .on_hover_text("Round the corners of the thumbnails")
                .changed()
            {
                self.gallery_round = round;
                vctx.reqs.push(Request::GalleryChanged);
            }

            let pictures = self.gallery_images();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let play = egui::Button::new(egui::RichText::new("Slideshow").size(11.5))
                    .fill(Color32::from_rgb(46, 53, 59));
                if ui
                    .add(play)
                    .on_hover_text("Play the pictures full screen, one second each (Space)")
                    .clicked()
                {
                    if let Some(first) = self.primary_selection().filter(|e| !e.is_dir && e.is_image()) {
                        vctx.reqs.push(Request::OpenViewer(first.path.clone(), true));
                    } else if let Some(path) = pictures.first() {
                        vctx.reqs.push(Request::OpenViewer(path.clone(), true));
                    }
                }
                let selected: Vec<PathBuf> = self
                    .sel
                    .iter()
                    .filter_map(|i| self.display_entries().get(*i))
                    .filter(|e| !e.is_dir && e.is_image())
                    .map(|e| e.path.clone())
                    .collect();
                let can_compare = selected.len() >= 2 || pictures.len() >= 2;
                ui.add_enabled_ui(can_compare, |ui| {
                    let btn = egui::Button::new(egui::RichText::new("Compare").size(11.5))
                        .fill(Color32::from_rgb(46, 53, 59));
                    if ui
                        .add(btn)
                        .on_hover_text("Put two pictures side by side, split or overlaid (C)")
                        .clicked()
                    {
                        let pair = if selected.len() >= 2 {
                            (selected[0].clone(), selected[1].clone())
                        } else {
                            (pictures[0].clone(), pictures[1].clone())
                        };
                        vctx.reqs.push(Request::OpenViewer(pair.0.clone(), false));
                        vctx.reqs.push(Request::Toast(format!(
                            "__compare__{}",
                            pair.1.display()
                        )));
                    }
                });
                ui.label(
                    egui::RichText::new(format!(
                        "{} picture{}",
                        pictures.len(),
                        if pictures.len() == 1 { "" } else { "s" }
                    ))
                    .size(11.0)
                    .color(TEXT_FAINT),
                );
            });
        });
    }

    /// The gallery itself: a grid of picture cards, in the pane's own sort
    /// order, with the pane's own selection rules.
    pub fn ui_pictures(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        egui::Frame::new()
            .fill(Color32::from_rgb(31, 34, 37))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                self.gallery_bar(ui, vctx);
            });

        // The wall holds the folder's pictures, plus its sub-folders so the
        // person can walk into the next folder of photographs. Documents are
        // left to the list views behind the gallery: a wall of letter icons is
        // not what the person asked for, and a 400 px tile of `notes.txt` is
        // nothing but an empty square.
        let entries: Vec<(usize, Entry)> = self
            .display_entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_dir || e.is_image())
            .map(|(i, e)| (i, e.clone()))
            .collect();
        if entries.is_empty() {
            let rect = ui.available_rect_before_wrap();
            ui.painter().text(
                Pos2::new(rect.center().x, rect.top() + 90.0),
                Align2::CENTER_CENTER,
                "No pictures in this folder",
                FontId::proportional(14.0),
                TEXT_DIM,
            );
            ui.painter().text(
                Pos2::new(rect.center().x, rect.top() + 112.0),
                Align2::CENTER_CENTER,
                "The gallery shows pictures and folders; the list views show every file.",
                FontId::proportional(12.0),
                TEXT_FAINT,
            );
            return;
        }

        let thumb = clamp_thumb(self.thumb_px);
        let card = card_size(thumb, self.gallery_names, self.gallery_dims);
        let avail = ui.available_width().max(card.x + 8.0);
        let mut cols = grid_columns(avail, card.x);
        let view_h = ui.available_height();
        let mut rows = grid_rows(entries.len(), cols);
        // A scroll bar takes its strip out of the inside width; pack again with
        // it removed so the last card of a row is never pushed under the bar.
        if rows as f32 * (card.y + THUMB_GAP) > view_h {
            cols = grid_columns((avail - 14.0).max(card.x), card.x);
            rows = grid_rows(entries.len(), cols);
        }

        let selected_colour = if vctx.active || !vctx.split {
            ROW_SEL_ACTIVE
        } else {
            ROW_SEL_INACTIVE
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, card.y + THUMB_GAP, rows, |ui, range| {
                ui.add_space(GRID_PAD * 0.5);
                for row in range {
                    ui.horizontal(|ui| {
                        ui.add_space(GRID_PAD * 0.5);
                        ui.spacing_mut().item_spacing.x = THUMB_GAP;
                        for c in 0..cols {
                            let idx = row * cols + c;
                            if idx >= entries.len() {
                                break;
                            }
                            let (disp, entry) = (entries[idx].0, entries[idx].1.clone());
                            let (rect, resp) =
                                ui.allocate_exact_size(card, Sense::click());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            if self.scroll_to == Some(disp) {
                                ui.scroll_to_rect(rect, Some(Align::Center));
                            }
                            let selected = self.sel.contains(&disp);
                            let bg = if selected {
                                selected_colour
                            } else if resp.hovered() {
                                ROW_HOVER
                            } else {
                                Color32::TRANSPARENT
                            };
                            if bg != Color32::TRANSPARENT {
                                ui.painter().rect_filled(
                                    rect.shrink(2.0),
                                    CornerRadius::same(6),
                                    bg,
                                );
                            }
                            self.draw_gallery_card(ui, rect, &entry, thumb, vctx);
                            self.handle_row_interaction(ui, rect, &resp, disp, vctx);
                        }
                    });
                }
                ui.add_space(GRID_PAD * 0.5);
            });
        if self.scroll_to.is_some() {
            self.scroll_to = None;
        }
    }

    fn draw_gallery_card(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        entry: &Entry,
        thumb: f32,
        vctx: &mut ViewCtx,
    ) {
        let pic_rect = Rect::from_min_size(
            Pos2::new(rect.center().x - thumb / 2.0, rect.top() + CELL_PAD),
            Vec2::splat(thumb),
        );
        ui.painter().rect_filled(
            pic_rect,
            CornerRadius::same(PICTURE_CORNER),
            Color32::from_rgb(24, 27, 30),
        );

        let mut drew = false;
        if !entry.is_dir && entry.is_image() && self.previews {
            let px = (thumb * ui.ctx().pixels_per_point()).round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px) {
                let src = tex.size();
                if src[0] > 0 && src[1] > 0 {
                    // A three-pixel mat keeps the picture clear of the card's
                    // rounded frame, so the frame reads as a frame even when the
                    // picture fills one side of its square.
                    let inner = (thumb - 6.0).max(8.0);
                    let scale = (inner / src[0] as f32)
                        .min(inner / src[1] as f32)
                        .min(1.0);
                    let size = Vec2::new(src[0] as f32 * scale, src[1] as f32 * scale);
                    let target = Rect::from_center_size(pic_rect.center(), size);
                    ui.painter().image(
                        tex.id(),
                        target,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    // "rounded image corner in thumbnail toggle": the picture
                    // takes the card's rounding when the switch is on, and its
                    // own square corners when it is off.
                    if self.gallery_round {
                        round_picture(
                            ui.painter(),
                            target,
                            PICTURE_CORNER as f32,
                            Color32::from_rgb(24, 27, 30),
                        );
                    }
                    drew = true;
                }
            }
        }
        if !drew {
            let side = (thumb * 0.45).clamp(28.0, 96.0);
            theme::draw_kind_icon_tinted(
                ui.painter(),
                Rect::from_center_size(pic_rect.center(), Vec2::splat(side)),
                if entry.is_symlink { Kind::Link } else { entry.kind },
                entry.broken,
                entry
                    .tint
                    .filter(|_| entry.is_dir)
                    .map(|c| Color32::from_rgb(c[0], c[1], c[2])),
            );
        }
        // The browser frames each picture with a two-pixel border; the same
        // frame here tells a very light picture apart from the dark grid. With
        // rounding switched off the frame keeps square corners to match.
        ui.painter().rect_stroke(
            pic_rect,
            CornerRadius::same(if self.gallery_round { PICTURE_CORNER } else { 2 }),
            Stroke::new(2.0, Color32::from_rgb(58, 64, 70)),
            egui::StrokeKind::Inside,
        );

        let mut y = pic_rect.bottom() + 4.0;
        if self.gallery_names {
            let max_w = rect.width() - 6.0;
            let font = FontId::proportional(12.0);
            let name = elide(ui, &entry.name, max_w, font.clone());
            ui.painter().text(
                Pos2::new(rect.center().x, y + 7.0),
                Align2::CENTER_CENTER,
                name,
                font,
                TEXT,
            );
            y += 17.0;
        }
        if self.gallery_dims {
            let size = if !entry.is_dir && entry.is_image() {
                probe_image_size(&entry.path)
            } else {
                None
            };
            let caption = if entry.is_dir {
                String::new()
            } else {
                dims_caption(size)
            };
            if !caption.is_empty() {
                ui.painter().text(
                    Pos2::new(rect.center().x, y + 7.0),
                    Align2::CENTER_CENTER,
                    elide(ui, &caption, rect.width() - 6.0, FontId::proportional(10.5)),
                    FontId::proportional(10.5),
                    TEXT_FAINT,
                );
            }
        }
    }
}

/// Mask the four corners of a drawn picture with `bg` so it looks rounded.
///
/// egui draws a texture as a plain rectangle, and the `image` crate's thumbnails
/// have opaque corners; masking them in the colour actually underneath (the
/// card's backing, or the viewer's near-black) is both exact and cheap. Each
/// corner is a fan of triangles from the outer corner around a quarter circle.
pub fn round_picture(painter: &egui::Painter, rect: Rect, radius: f32, bg: Color32) {
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    if r < 0.5 {
        return;
    }
    let corners = [
        (
            rect.left_top(),
            Pos2::new(rect.left() + r, rect.top() + r),
            std::f32::consts::PI,
        ),
        (
            rect.right_top(),
            Pos2::new(rect.right() - r, rect.top() + r),
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            rect.right_bottom(),
            Pos2::new(rect.right() - r, rect.bottom() - r),
            0.0,
        ),
        (
            rect.left_bottom(),
            Pos2::new(rect.left() + r, rect.bottom() - r),
            std::f32::consts::FRAC_PI_2,
        ),
    ];
    const STEPS: usize = 7;
    for (corner, centre, start) in corners {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(corner, bg);
        for i in 0..=STEPS {
            let a = start + std::f32::consts::FRAC_PI_2 * (i as f32 / STEPS as f32);
            let p = Pos2::new(centre.x + r * a.cos(), centre.y + r * a.sin());
            mesh.colored_vertex(p, bg);
            if i > 0 {
                mesh.add_triangle(0, i as u32, i as u32 + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// The prompt block shared by the viewer's side panel and the information panel:
/// the fields first (Steps, Sampler, Seed…), then the prompt itself in a
/// read-only scroll box, then the negative prompt, then where it came from.
pub fn prompt_panel(ui: &mut egui::Ui, meta: Option<&crate::imagemeta::ImageMeta>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Prompt")
                .size(12.0)
                .strong()
                .color(TEXT),
        );
        if let Some(m) = meta {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Copy").small())
                    .on_hover_text("Copy the prompt to the clipboard")
                    .clicked()
                {
                    ui.ctx().copy_text(m.copy_text());
                }
                ui.label(
                    egui::RichText::new(&m.source)
                        .size(9.5)
                        .color(TEXT_FAINT),
                );
            });
        }
    });
    ui.add_space(2.0);
    let Some(m) = meta else {
        ui.label(
            egui::RichText::new("No generation prompt is stored in this picture.")
                .size(11.5)
                .color(TEXT_DIM),
        );
        ui.label(
            egui::RichText::new("Pictures made by common text-to-image tools carry theirs; photos usually do not.")
                .size(10.5)
                .color(TEXT_FAINT),
        );
        return;
    };
    for (k, v) in &m.fields {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{k}:")).size(10.5).color(TEXT_DIM));
            ui.label(
                egui::RichText::new(elide(ui, v, 190.0, FontId::proportional(11.0)))
                    .size(11.0)
                    .strong()
                    .color(TEXT),
            );
        });
    }
    if !m.fields.is_empty() {
        ui.add_space(3.0);
    }
    let text = m.prompt.clone().unwrap_or_else(|| m.raw.clone());
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut text = text.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Small)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
    if let Some(neg) = &m.negative {
        ui.add_space(3.0);
        ui.label(
            egui::RichText::new("Negative prompt")
                .size(10.0)
                .color(TEXT_FAINT),
        );
        ui.label(egui::RichText::new(neg).size(10.5).color(TEXT_DIM));
    }
}

/// Shorten a name to `max_w` with an ellipsis, measured with the same font it
/// is drawn in.
fn elide(ui: &egui::Ui, text: &str, max_w: f32, font: FontId) -> String {
    let painter = ui.painter();
    let full = painter.layout_no_wrap(text.to_owned(), font.clone(), TEXT);
    if full.size().x <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut trial = out.clone();
        trial.push(ch);
        trial.push('…');
        let w = painter
            .layout_no_wrap(trial.clone(), font.clone(), TEXT)
            .size()
            .x;
        if w > max_w {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// The full-window viewer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareMode {
    Side,
    Split,
    Overlay,
}

impl CompareMode {
    pub fn label(self) -> &'static str {
        match self {
            CompareMode::Side => "Side",
            CompareMode::Split => "Split",
            CompareMode::Overlay => "Overlay",
        }
    }
    /// Compare is chosen by name rather than by mark: "side", "split" and
    /// "overlay" are the browser's own words for the three.
    pub fn glyph(self) -> &'static str {
        ""
    }
}

pub struct Compare {
    pub a: PathBuf,
    pub b: PathBuf,
    pub mode: CompareMode,
    pub opacity: u8,
    pub sync: bool,
    pub zoom_b: f32,
    pub pan_b: Vec2,
    pub split_at: f32,
}

pub struct Slide {
    pub interval: f32,
    pub fade: bool,
    pub next_at: Instant,
    pub started: Instant,
}

impl Slide {
    pub fn new(interval: f32) -> Self {
        Self {
            interval: clamp_slide(interval),
            fade: true,
            next_at: Instant::now() + Duration::from_secs_f32(clamp_slide(interval)),
            started: Instant::now(),
        }
    }

    /// Whether a picture is due. Returns it so the caller advances by one.
    pub fn tick(&mut self, now: Instant) -> bool {
        if now >= self.next_at {
            let step = Duration::from_secs_f32(self.interval);
            self.next_at = now + step;
            true
        } else {
            false
        }
    }
}

pub struct Viewer {
    pub dir: PathBuf,
    pub items: Vec<PathBuf>,
    pub index: usize,
    pub zoom: f32,
    pub pan: Vec2,
    /// Fit the picture to the window rather than showing it at its own size.
    pub fit: bool,
    pub show_info: bool,
    pub slide: Option<Slide>,
    pub compare: Option<Compare>,
    /// When the picture on screen changed, so it can fade in over 300 ms.
    pub shown_at: Instant,
    pub show_help: bool,
    /// Fill the screen: on when the viewer was started as a slideshow, and
    /// switchable with F. `fullscreen_done` is what the window has been told, so
    /// the command is sent once per change rather than every frame.
    pub fullscreen: bool,
    pub fullscreen_done: bool,
    /// The prompt read for the picture on screen, with the path it belongs to.
    pub meta: Option<(PathBuf, Option<crate::imagemeta::ImageMeta>)>,
    /// The prompt panel opens itself once, the first time a picture that has a
    /// prompt comes up — the person asked for the prompt to be read, so it
    /// should not be hidden behind a toggle they have to find.
    pub prompt_opened: bool,
}

impl Viewer {
    pub fn new(dir: PathBuf, items: Vec<PathBuf>, index: usize, play: bool) -> Self {
        let mut v = Self {
            dir,
            items,
            index,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            show_info: false,
            slide: None,
            compare: None,
            shown_at: Instant::now(),
            show_help: false,
            fullscreen: play,
            fullscreen_done: false,
            meta: None,
            prompt_opened: false,
        };
        if play {
            v.slide = Some(Slide::new(SLIDE_DEFAULT));
        }
        v
    }

    pub fn current(&self) -> Option<&PathBuf> {
        self.items.get(self.index)
    }

    pub fn name(&self) -> String {
        self.current()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "No picture".to_string())
    }

    pub fn step(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as i32;
        self.index = (((self.index as i32 + delta) % n + n) % n) as usize;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.fit = true;
        self.shown_at = Instant::now();
        if let Some(slide) = self.slide.as_mut() {
            slide.next_at = Instant::now() + Duration::from_secs_f32(slide.interval);
        }
    }

    /// The pair to compare: what was asked for, or this picture against the next
    /// one round the folder.
    pub fn start_compare(&mut self) -> bool {
        if self.items.len() < 2 {
            return false;
        }
        let a = self.current().cloned().unwrap_or_default();
        let b = self
            .items
            .get((self.index + 1) % self.items.len())
            .cloned()
            .unwrap_or_else(|| a.clone());
        self.compare = Some(Compare {
            a,
            b,
            mode: CompareMode::Side,
            opacity: OPACITY_DEFAULT,
            sync: true,
            zoom_b: 1.0,
            pan_b: Vec2::ZERO,
            split_at: 0.5,
        });
        self.slide = None;
        true
    }
}

/// The viewer's own drawing helpers, kept out of `impl App` so the borrow of the
/// viewer stays small.
fn bar_button(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
    // The glyphs are drawn with the same little line-shapes the toolbar uses,
    // so no font is ever missing a symbol.
    let font = FontId::proportional(12.0);
    let text_w = if label.is_empty() {
        0.0
    } else {
        ui.painter()
            .layout_no_wrap(label.to_owned(), font.clone(), TEXT)
            .size()
            .x
    };
    let width = if glyph.is_empty() {
        text_w + 14.0
    } else {
        26.0 + if label.is_empty() { 0.0 } else { text_w + 4.0 }
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(24.0), 22.0), Sense::click());
    let tint = if active {
        Color32::WHITE
    } else if resp.hovered() {
        TEXT
    } else {
        TEXT_DIM
    };
    if active || resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(3),
            if active {
                Color32::from_rgb(52, 60, 68)
            } else {
                Color32::from_rgb(40, 46, 52)
            },
        );
    }
    if active {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if glyph.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            font,
            tint,
        );
    } else {
        let glyph_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 13.0, rect.center().y),
            Vec2::splat(18.0),
        );
        theme::draw_glyph(ui.painter(), glyph_rect, glyph, tint);
        if !label.is_empty() {
            ui.painter().text(
                Pos2::new(rect.left() + 24.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font,
                tint,
            );
        }
    }
    resp
}

impl crate::App {
    /// Open the viewer on a picture, with the pane's pictures as the run to page
    /// through.
    pub fn open_viewer(&mut self, path: &Path, play: bool) {
        let pane = self.pane();
        let mut items = pane.gallery_images();
        if items.len() < 2 {
            // A picture named on the command line opens the viewer on the very
            // first frame, before the folder has been read, so the pane's list
            // holds nothing yet. Read the folder here instead: paging and the
            // comparison then have the whole set straight away.
            if let Some(dir) = path.parent() {
                let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .map(|e| e.to_string_lossy().to_lowercase())
                                .map(|e| {
                                    matches!(
                                        e.as_str(),
                                        "png" | "jpg"
                                            | "jpeg"
                                            | "gif"
                                            | "bmp"
                                            | "webp"
                                            | "tif"
                                            | "tiff"
                                    )
                                })
                                .unwrap_or(false)
                    })
                    .collect();
                found.sort();
                if found.len() > items.len() {
                    items = found;
                }
            }
        }
        if items.is_empty() {
            items.push(path.to_path_buf());
        }
        let index = items.iter().position(|p| p == path).unwrap_or(0);
        let dir = pane.current_dir();
        // The interval and the fade come from the last time the person set them,
        // so a show starts the way they left it.
        let interval = clamp_slide(self.config.slide_secs);
        let fade = self.config.slide_fade;
        let mut viewer = Viewer::new(dir, items, index, play);
        if let Some(slide) = viewer.slide.as_mut() {
            slide.interval = interval;
            slide.fade = fade;
            slide.next_at = Instant::now() + Duration::from_secs_f32(interval);
        }
        self.viewer = Some(viewer);
    }

    /// Keys the viewer owns while it is open: arrows page, Space plays, + and −
    /// zoom, Esc closes. Returned as `true` when the key belonged to the viewer.
    pub fn viewer_keys(&mut self, ctx: &egui::Context) -> bool {
        let mut close = false;
        let mut needs_two = false;
        let mut fullscreen = false;
        {
            let Some(viewer) = self.viewer.as_mut() else {
                return false;
            };
            ctx.input_mut(|i| {
                use egui::Key;
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    close = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowRight) {
                    viewer.step(1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                    viewer.step(-1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Space) {
                    if viewer.slide.is_some() {
                        viewer.slide = None;
                    } else {
                        viewer.slide = Some(Slide::new(SLIDE_DEFAULT));
                    }
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Plus)
                    || i.consume_key(egui::Modifiers::NONE, Key::Equals)
                {
                    viewer.zoom = zoom_step(viewer.zoom, ZOOM_BUTTON_STEP);
                    viewer.fit = false;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Minus) {
                    viewer.zoom = zoom_step(viewer.zoom, 1.0 / ZOOM_BUTTON_STEP);
                    viewer.fit = false;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Num0) {
                    viewer.zoom = 1.0;
                    viewer.pan = Vec2::ZERO;
                    viewer.fit = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::C) && !viewer.start_compare() {
                    needs_two = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::I) {
                    viewer.show_info = !viewer.show_info;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::F) {
                    viewer.fullscreen = !viewer.fullscreen;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Questionmark) {
                    viewer.show_help = !viewer.show_help;
                }
            });
            fullscreen = viewer.fullscreen;
        }
        if needs_two {
            self.toast("A comparison needs two pictures in this folder.");
        }
        if close {
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            self.viewer = None;
        }
        true
    }

    /// Draw the viewer over the whole window. Called after the normal chrome so
    /// it covers the tab strip, the panels and the status bar, exactly as the
    /// browser's `#viewer` covers its page.
    pub fn viewer_ui(&mut self, ctx: &egui::Context) {
        if self.viewer.is_none() {
            return;
        }
        let screen = ctx.screen_rect();
        let mut close = false;
        let mut open_compare_with_next = false;
        let mut want_compare_close = false;

        // Keep the frame clock ticking while a slideshow runs or a fade is
        // under way, and load whatever picture is on screen.
        let now = Instant::now();
        let mut slide_due = false;
        let mut slide_interval = SLIDE_DEFAULT;
        let mut fade = false;
        if let Some(v) = self.viewer.as_ref() {
            if let Some(s) = &v.slide {
                slide_interval = s.interval;
                fade = s.fade;
            }
        }

        // "start full screen slideshow for images": the window is told once per
        // change, whether the viewer opened as a slideshow or the person pressed
        // F. Sending it every frame would make the window manager flap.
        if let Some(v) = self.viewer.as_mut() {
            if v.fullscreen != v.fullscreen_done {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(v.fullscreen));
                v.fullscreen_done = v.fullscreen;
            }
        }

        // "extract meta data from prompt": read the picture's own recipe once per
        // picture, not once per frame.
        if let Some(v) = self.viewer.as_mut() {
            if let Some(path) = v.current().cloned() {
                if !matches!(&v.meta, Some((p, _)) if *p == path) {
                    v.meta = Some((path.clone(), crate::imagemeta::read(&path)));
                }
                let has_prompt = v
                    .meta
                    .as_ref()
                    .and_then(|(_, m)| m.as_ref())
                    .map(|m| !m.is_empty())
                    .unwrap_or(false);
                if has_prompt && !v.prompt_opened {
                    v.show_info = true;
                    v.prompt_opened = true;
                }
            }
        }

        egui::Area::new(Id::new("picture_viewer"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_rgb(10, 12, 14));

                let top = Rect::from_min_size(
                    screen.min,
                    Vec2::new(screen.width(), 46.0),
                );
                let bottom = Rect::from_min_size(
                    Pos2::new(screen.left(), screen.bottom() - 44.0),
                    Vec2::new(screen.width(), 44.0),
                );
                let canvas = Rect::from_min_max(
                    Pos2::new(screen.left(), top.bottom()),
                    Pos2::new(screen.right(), bottom.top()),
                );

                // A picture this size is what the browser shows behind its
                // viewer: near-black, so the photograph is the bright thing.
                ui.painter()
                    .rect_filled(canvas, 0.0, Color32::from_rgb(14, 16, 18));

                let viewer = self.viewer.as_mut().unwrap();
                let path = viewer.current().cloned().unwrap_or_default();
                let tex = self.big.get(ctx, &path);

                // ---- picture area ----------------------------------------
                let drag = ui.interact(canvas, Id::new("viewer_canvas"), Sense::click_and_drag());
                if drag.dragged() {
                    viewer.pan += drag.drag_delta();
                }
                if drag.double_clicked() {
                    if viewer.zoom > 1.01 {
                        viewer.zoom = 1.0;
                        viewer.pan = Vec2::ZERO;
                        viewer.fit = true;
                    } else {
                        viewer.zoom = 2.0;
                        viewer.fit = false;
                    }
                }
                if drag.hovered() {
                    let wheel = ui.input(|i| i.smooth_scroll_delta.y + i.raw_scroll_delta.y);
                    if wheel.abs() > 0.5 {
                        let factor = if wheel > 0.0 { ZOOM_WHEEL_IN } else { ZOOM_WHEEL_OUT };
                        let old = viewer.zoom;
                        viewer.zoom = zoom_step(viewer.zoom, factor);
                        let ratio = viewer.zoom / old;
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let off = pos - canvas.center();
                            viewer.pan = off * (1.0 - ratio) + viewer.pan * ratio;
                        }
                        viewer.fit = false;
                    }
                }

                if let Some(tex) = tex.clone() {
                    let src = tex.size();
                    if src[0] > 0 && src[1] > 0 {
                        if let Some(cmp) = viewer.compare.as_mut() {
                            draw_compare(
                                ui,
                                canvas,
                                &tex,
                                &mut viewer.zoom,
                                &mut viewer.pan,
                                cmp,
                                &mut self.big,
                                ctx,
                            );
                        } else {
                            let fit = (canvas.width() / src[0] as f32)
                                .min(canvas.height() / src[1] as f32);
                            let scale = if viewer.fit { fit } else { fit * viewer.zoom };
                            let size = Vec2::new(src[0] as f32 * scale, src[1] as f32 * scale);
                            let max_pan = Vec2::new(
                                ((size.x - canvas.width()) * 0.5).max(0.0),
                                ((size.y - canvas.height()) * 0.5).max(0.0),
                            );
                            viewer.pan.x = viewer.pan.x.clamp(-max_pan.x, max_pan.x);
                            viewer.pan.y = viewer.pan.y.clamp(-max_pan.y, max_pan.y);
                            let alpha = if fade && viewer.slide.is_some() {
                                let elapsed =
                                    now.duration_since(viewer.shown_at).as_millis() as f32
                                        / FADE_MS as f32;
                                (elapsed.min(1.0) * 255.0) as u8
                            } else {
                                255
                            };
                            ui.painter().image(
                                tex.id(),
                                Rect::from_center_size(canvas.center() + viewer.pan, size),
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::from_white_alpha(alpha),
                            );
                        }
                    }
                } else {
                    ui.painter().text(
                        canvas.center(),
                        Align2::CENTER_CENTER,
                        "Loading picture…",
                        FontId::proportional(14.0),
                        TEXT_DIM,
                    );
                }

                // ---- top bar ---------------------------------------------
                ui.painter()
                    .rect_filled(top, 0.0, Color32::from_rgb(20, 23, 26));
                ui.painter().line_segment(
                    [Pos2::new(top.left(), top.bottom()), Pos2::new(top.right(), top.bottom())],
                    Stroke::new(1.0, Color32::from_rgb(38, 43, 48)),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(top.shrink2(Vec2::new(10.0, 6.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if bar_button(ui, "close", "", false)
                            .on_hover_text("Close (Esc)")
                            .clicked()
                        {
                            close = true;
                        }
                        let name = viewer.name();
                        let shown = elide(ui, &name, 420.0, FontId::proportional(13.0));
                        ui.label(egui::RichText::new(shown).size(13.0).color(TEXT));
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                viewer.index + 1,
                                viewer.items.len()
                            ))
                            .size(11.0)
                            .color(TEXT_FAINT),
                        );

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if bar_button(ui, "info", "", viewer.show_info)
                                .on_hover_text("Picture details (I)")
                                .clicked()
                            {
                                viewer.show_info = !viewer.show_info;
                            }
                            if bar_button(ui, "", "?", viewer.show_help)
                                .on_hover_text("Keys (?)")
                                .clicked()
                            {
                                viewer.show_help = !viewer.show_help;
                            }
                            if bar_button(
                                ui,
                                "",
                                if viewer.fullscreen { "windowed" } else { "full screen" },
                                viewer.fullscreen,
                            )
                            .on_hover_text("Fill the screen (F)")
                            .clicked()
                            {
                                viewer.fullscreen = !viewer.fullscreen;
                            }
                            ui.add_space(4.0);
                            let playing = viewer.slide.is_some();
                            if bar_button(ui, if playing { "pause" } else { "play" }, "", playing)
                                .on_hover_text("Slideshow (Space)")
                                .clicked()
                            {
                                if playing {
                                    viewer.slide = None;
                                } else {
                                    viewer.slide = Some(Slide::new(SLIDE_DEFAULT));
                                }
                            }
                            if let Some(slide) = viewer.slide.as_mut() {
                                let mut secs = slide.interval;
                                let slider = ui.add_sized(
                                    Vec2::new(64.0, 16.0),
                                    egui::Slider::new(
                                        &mut secs,
                                        SLIDE_MIN..=10.0,
                                    )
                                    .show_value(false)
                                    .step_by(SLIDE_STEP as f64),
                                );
                                if slider.changed() {
                                    slide.interval = clamp_slide(secs);
                                    slide.next_at = now
                                        + Duration::from_secs_f32(slide.interval);
                                }
                                ui.label(
                                    egui::RichText::new(format!("{:.1}s", slide.interval))
                                        .size(10.5)
                                        .color(TEXT_FAINT),
                                );
                                let mut fade_on = slide.fade;
                                if ui
                                    .add(egui::Checkbox::new(
                                        &mut fade_on,
                                        egui::RichText::new("Fade").size(11.0),
                                    ))
                                    .changed()
                                {
                                    slide.fade = fade_on;
                                }
                            }
                            ui.add_space(4.0);
                            let comparing = viewer.compare.is_some();
                            if bar_button(ui, "compare", "Compare", comparing)
                                .on_hover_text("Compare with the next picture (C)")
                                .clicked()
                            {
                                if comparing {
                                    want_compare_close = true;
                                } else {
                                    open_compare_with_next = true;
                                }
                            }
                            ui.add_space(4.0);
                            // Added right to left, so they read "− 100 % +" on
                            // screen, the way the browser's compare bar does.
                            if bar_button(ui, "plus", "", false).clicked() {
                                viewer.zoom = zoom_step(viewer.zoom, ZOOM_BUTTON_STEP);
                                viewer.fit = false;
                            }
                            let pct = (viewer.zoom * 100.0).round() as u32;
                            ui.label(
                                egui::RichText::new(format!("{pct}%"))
                                    .size(11.0)
                                    .color(TEXT_DIM),
                            );
                            if bar_button(ui, "minus", "", false).clicked() {
                                viewer.zoom = zoom_step(viewer.zoom, 1.0 / ZOOM_BUTTON_STEP);
                                viewer.fit = false;
                            }
                            if bar_button(ui, "fit", "Fit", viewer.fit)
                                .on_hover_text("Fit to window (0)")
                                .clicked()
                            {
                                viewer.zoom = 1.0;
                                viewer.pan = Vec2::ZERO;
                                viewer.fit = true;
                            }
                        });
                    },
                );

                // ---- bottom bar ------------------------------------------
                ui.painter()
                    .rect_filled(bottom, 0.0, Color32::from_rgb(20, 23, 26));
                ui.painter().line_segment(
                    [
                        Pos2::new(bottom.left(), bottom.top()),
                        Pos2::new(bottom.right(), bottom.top()),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(38, 43, 48)),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(bottom.shrink2(Vec2::new(10.0, 6.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if bar_button(ui, "chevron_left", "Previous", false)
                            .on_hover_text("Previous picture (←)")
                            .clicked()
                        {
                            viewer.step(-1);
                        }
                        if bar_button(ui, "chevron_right", "Next", false)
                            .on_hover_text("Next picture (→)")
                            .clicked()
                        {
                            viewer.step(1);
                        }
                        ui.add_space(6.0);
                        let caption = match probe_image_size(&path) {
                            Some((w, h)) => format!(
                                "{} × {} · {} · {} · {}",
                                w,
                                h,
                                ratio_label(w, h),
                                megapixels(w, h),
                                fsutil::format_size(file_size(&path))
                            ),
                            None => fsutil::format_size(file_size(&path)),
                        };
                        if viewer.show_info {
                            let entry = Entry::from_path(&path);
                            let long = format!(
                                "{} · {} · modified {}",
                                caption,
                                entry.type_label(),
                                fsutil::human_date(entry.modified)
                            );
                            ui.label(
                                egui::RichText::new(elide(ui, &long, 700.0, FontId::proportional(11.5)))
                                    .size(11.5)
                                    .color(TEXT_DIM),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(caption)
                                    .size(11.5)
                                    .color(TEXT_FAINT),
                            );
                        }

                        if let Some(cmp) = viewer.compare.as_mut() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                if bar_button(ui, "swap", "Swap", false).clicked() {
                                    std::mem::swap(&mut cmp.a, &mut cmp.b);
                                }
                                let mut sync = cmp.sync;
                                if ui
                                    .add(egui::Checkbox::new(
                                        &mut sync,
                                        egui::RichText::new("Sync").size(11.0),
                                    ))
                                    .changed()
                                {
                                    cmp.sync = sync;
                                    if sync {
                                        cmp.zoom_b = viewer.zoom;
                                    }
                                }
                                for mode in [
                                    CompareMode::Overlay,
                                    CompareMode::Split,
                                    CompareMode::Side,
                                ] {
                                    if bar_button(ui, mode.glyph(), mode.label(), cmp.mode == mode)
                                        .clicked()
                                    {
                                        cmp.mode = mode;
                                    }
                                }
                                if cmp.mode == CompareMode::Overlay {
                                    let mut op = cmp.opacity as f32;
                                    let slider = ui.add_sized(
                                        Vec2::new(88.0, 16.0),
                                        egui::Slider::new(&mut op, 0.0..=100.0)
                                            .show_value(false)
                                            .suffix("%"),
                                    );
                                    if slider.changed() {
                                        cmp.opacity = op.round() as u8;
                                    }
                                    ui.label(
                                        egui::RichText::new(format!("{}%", cmp.opacity))
                                            .size(10.5)
                                            .color(TEXT_FAINT),
                                    );
                                    ui.label(
                                        egui::RichText::new("Opacity")
                                            .size(11.0)
                                            .color(TEXT_DIM),
                                    );
                                }
                            });
                        }
                    },
                );

                if viewer.show_help {
                    let help = "Esc close   ← → page   Space slideshow   + − zoom   0 fit   I details   F full screen   C compare";
                    ui.painter().text(
                        Pos2::new(canvas.center().x, canvas.top() + 18.0),
                        Align2::CENTER_CENTER,
                        help,
                        FontId::proportional(11.5),
                        TEXT_FAINT,
                    );
                }

                // "extract meta data from prompt": the same block the
                // information panel shows, laid over the picture so the person
                // can read the prompt without leaving the slideshow.
                if viewer.show_info {
                    let panel = Rect::from_min_max(
                        Pos2::new(canvas.right() - 312.0, canvas.top() + 10.0),
                        Pos2::new(canvas.right() - 10.0, canvas.bottom() - 10.0),
                    );
                    ui.painter().rect_filled(
                        panel,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(22, 26, 30, 236),
                    );
                    ui.painter().rect_stroke(
                        panel,
                        CornerRadius::same(6),
                        Stroke::new(1.0, Color32::from_rgb(48, 54, 60)),
                        egui::StrokeKind::Inside,
                    );
                    let inner = panel.shrink(10.0);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.set_clip_rect(inner);
                            prompt_panel(ui, viewer.meta.as_ref().and_then(|(_, m)| m.as_ref()));
                        },
                    );
                }

                if let Some(slide) = viewer.slide.as_mut() {
                    if slide.tick(now) {
                        slide_due = true;
                    }
                }
            });

        if slide_due {
            if let Some(v) = self.viewer.as_mut() {
                v.step(1);
            }
        }
        if close {
            if let Some(v) = self.viewer.as_ref() {
                if v.fullscreen {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                }
            }
            self.viewer = None;
        }
        if want_compare_close {
            if let Some(v) = self.viewer.as_mut() {
                v.compare = None;
            }
        }
        if open_compare_with_next {
            let started = self
                .viewer
                .as_mut()
                .map(|v| v.start_compare())
                .unwrap_or(false);
            if !started {
                self.toast("A comparison needs two pictures in this folder.");
            }
        }
        if let Some(v) = self.viewer.as_ref() {
            if v.slide.is_some() {
                ctx.request_repaint_after(Duration::from_millis(50));
                let _ = slide_interval;
            }
        }
    }
}

/// Draw two pictures side by side, split at a divider, or one over the other.
#[allow(clippy::too_many_arguments)]
fn draw_compare(
    ui: &mut egui::Ui,
    canvas: Rect,
    tex_a: &egui::TextureHandle,
    zoom_a: &mut f32,
    pan_a: &mut Vec2,
    cmp: &mut Compare,
    big: &mut BigCache,
    ctx: &egui::Context,
) {
    let tex_b = big.get(ctx, &cmp.b);
    let painter = ui.painter();
    match cmp.mode {
        CompareMode::Side => {
            let gap = 8.0;
            let half = (canvas.width() - gap) * 0.5;
            let left = Rect::from_min_size(canvas.min, Vec2::new(half, canvas.height()));
            let right = Rect::from_min_size(
                Pos2::new(canvas.left() + half + gap, canvas.top()),
                Vec2::new(half, canvas.height()),
            );
            draw_fit(ui, left, tex_a, *zoom_a, *pan_a);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(left.right(), canvas.top()),
                    Vec2::new(gap, canvas.height()),
                ),
                0.0,
                Color32::from_rgb(30, 34, 38),
            );
            if let Some(tb) = tex_b.as_ref() {
                let zb = if cmp.sync { *zoom_a } else { cmp.zoom_b };
                draw_fit(ui, right, tb, zb, cmp.pan_b);
            } else {
                painter.text(
                    right.center(),
                    Align2::CENTER_CENTER,
                    "Loading…",
                    FontId::proportional(12.0),
                    TEXT_DIM,
                );
            }
            // A dark strip behind each name, otherwise the name is written on
            // top of the photograph and cannot be read.
            for (pane, name) in [(left, file_label(&cmp.a)), (right, file_label(&cmp.b))] {
                let strip = Rect::from_min_size(
                    Pos2::new(pane.left(), pane.bottom() - 22.0),
                    Vec2::new(pane.width(), 22.0),
                );
                painter.rect_filled(strip, 0.0, Color32::from_black_alpha(150));
                painter.text(
                    strip.center(),
                    Align2::CENTER_CENTER,
                    name,
                    FontId::proportional(10.5),
                    Color32::from_rgb(226, 232, 238),
                );
            }
        }
        CompareMode::Split => {
            let x = canvas.left() + canvas.width() * cmp.split_at;
            let left = Rect::from_min_max(canvas.min, Pos2::new(x, canvas.bottom()));
            let right = Rect::from_min_max(Pos2::new(x, canvas.top()), canvas.max);
            // Both pictures keep the same framing, as in the browser: the
            // divider only decides which one is visible on each side.
            draw_fit_clipped(ui, canvas, left, tex_a, *zoom_a, *pan_a);
            if let Some(tb) = tex_b.as_ref() {
                draw_fit_clipped(ui, canvas, right, tb, *zoom_a, *pan_a);
            }
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(x - 1.0, canvas.top()), Vec2::new(2.0, canvas.height())),
                0.0,
                Color32::from_rgb(220, 226, 232),
            );
            let handle = ui.interact(
                Rect::from_min_size(
                    Pos2::new(x - 10.0, canvas.top()),
                    Vec2::new(20.0, canvas.height()),
                ),
                Id::new("compare_divider"),
                Sense::drag(),
            );
            if handle.dragged() {
                if let Some(pos) = handle.interact_pointer_pos() {
                    cmp.split_at = ((pos.x - canvas.left()) / canvas.width()).clamp(0.05, 0.95);
                }
            }
        }
        CompareMode::Overlay => {
            draw_fit(ui, canvas, tex_a, *zoom_a, *pan_a);
            if let Some(tb) = tex_b.as_ref() {
                let alpha = cmp.opacity as f32 / 100.0;
                draw_fit_alpha(ui, canvas, tb, *zoom_a, *pan_a, alpha);
            }
            painter.text(
                Pos2::new(canvas.left() + 12.0, canvas.top() + 14.0),
                Align2::LEFT_CENTER,
                format!("{} + {} ({}%)", file_label(&cmp.a), file_label(&cmp.b), cmp.opacity),
                FontId::proportional(10.5),
                TEXT_DIM,
            );
        }
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn fit_rect(canvas: Rect, tex: &egui::TextureHandle, zoom: f32, pan: Vec2) -> Rect {
    let src = tex.size();
    if src[0] == 0 || src[1] == 0 {
        return Rect::from_center_size(canvas.center(), Vec2::ZERO);
    }
    let fit = (canvas.width() / src[0] as f32).min(canvas.height() / src[1] as f32);
    let scale = fit * zoom.max(0.01);
    Rect::from_center_size(
        canvas.center() + pan,
        Vec2::new(src[0] as f32 * scale, src[1] as f32 * scale),
    )
}

fn draw_fit(ui: &egui::Ui, canvas: Rect, tex: &egui::TextureHandle, zoom: f32, pan: Vec2) {
    ui.painter().image(
        tex.id(),
        fit_rect(canvas, tex, zoom, pan),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn draw_fit_alpha(
    ui: &egui::Ui,
    canvas: Rect,
    tex: &egui::TextureHandle,
    zoom: f32,
    pan: Vec2,
    alpha: f32,
) {
    ui.painter().image(
        tex.id(),
        fit_rect(canvas, tex, zoom, pan),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8),
    );
}

fn draw_fit_clipped(
    ui: &egui::Ui,
    canvas: Rect,
    clip: Rect,
    tex: &egui::TextureHandle,
    zoom: f32,
    pan: Vec2,
) {
    let painter = ui.painter().with_clip_rect(clip);
    painter.image(
        tex.id(),
        fit_rect(canvas, tex, zoom, pan),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}
