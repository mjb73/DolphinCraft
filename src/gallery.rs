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
//! | slideshow interval | 1.5 s first (the person's choice), 0.5 s steps | the person asked for 1.5 s; the browser's `#slide-interval` starts at 1 s (`min="0.5" value="1" step="0.5"`) |
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
use crate::theme;
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
pub const PICTURE_CORNER: u8 = 10;
/// Padding around the whole grid.
pub const GRID_PAD: f32 = 16.0;

/// Viewer zoom limits and steps.
pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 20.0;
pub const ZOOM_BUTTON_STEP: f32 = 1.25;
pub const ZOOM_WHEEL_IN: f32 = 1.1;
pub const ZOOM_WHEEL_OUT: f32 = 0.9;

/// Slideshow: first interval, smallest step, fade length. The browser starts
/// its own show at 1 s; the person asked for 1.5 s here, and a fresh window (or
/// one whose settings file predates the interval) uses that.
pub const SLIDE_DEFAULT: f32 = 1.5;
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

/// Zoom by `factor` about `anchor` — a point measured from the middle of the
/// pane the picture is drawn in — so whatever sits under the pointer stays
/// under it. Without the anchor the picture slides away from the cursor, which
/// is what made zooming feel like it was jumping.
pub fn zoom_about(zoom: f32, pan: Vec2, anchor: Vec2, factor: f32) -> (f32, Vec2) {
    let z = zoom_step(zoom, factor);
    let ratio = z / zoom.max(0.0001);
    (z, anchor - (anchor - pan) * ratio)
}

/// Keep a picture inside its pane: while it is bigger than the pane it may be
/// pushed until an edge reaches the pane's edge, and while it still fits it
/// does not move at all. Before this a drag could throw a picture right out of
/// the window with no way back but Fit.
pub fn clamp_pan(pan: Vec2, size: Vec2, pane: Vec2) -> Vec2 {
    let max = Vec2::new(
        ((size.x - pane.x) * 0.5).max(0.0),
        ((size.y - pane.y) * 0.5).max(0.0),
    );
    Vec2::new(pan.x.clamp(-max.x, max.x), pan.y.clamp(-max.y, max.y))
}

/// The size a picture is drawn at: its own pixels scaled to fit the pane, then
/// by the zoom. The same fit rule as `KPixmapModifier::scale` in the original.
pub fn fitted_size(src: [usize; 2], zoom: f32, pane: Vec2) -> Vec2 {
    if src[0] == 0 || src[1] == 0 || pane.x <= 0.0 || pane.y <= 0.0 {
        return Vec2::ZERO;
    }
    let fit = (pane.x / src[0] as f32).min(pane.y / src[1] as f32);
    Vec2::new(src[0] as f32, src[1] as f32) * fit * zoom.max(0.01)
}

/// One frame's wheel events in the units `state::wheel_zoom_steps` counts (120
/// per notch). The smoothed delta is deliberately not used: egui spreads one
/// notch over a dozen frames, and counting each of those bought a zoom step on
/// every frame — two notches ran the zoom straight to its 2000 % ceiling.
pub fn wheel_units_in_frame(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        let mut line = 0.0f32;
        let mut pixels = 0.0f32;
        for e in &i.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = e {
                match unit {
                    egui::MouseWheelUnit::Line => {
                        // One notch of a real wheel reaches the window as a press
                        // and a release, two events microseconds apart. Counting
                        // both doubled every notch, so a frame counts as one notch
                        // however many line events it carried.
                        if delta.y.abs() > line.abs() {
                            line = delta.y;
                        }
                    }
                    other => pixels += crate::state::wheel_units(*other, delta.y),
                }
            }
        }
        line * crate::state::WHEEL_UNITS_PER_STEP + pixels
    })
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

    /// True once a picture has been tried and the decoder refused it, so the
    /// viewer can say so instead of waiting on it for ever.
    pub fn has_failed(&self, path: &Path) -> bool {
        self.failed.iter().any(|p| p == path)
    }
}

// ---------------------------------------------------------------------------
// Gallery cards
// ---------------------------------------------------------------------------

/// The one font every caption under a picture is drawn in — the file name and,
/// when switched on, the picture's own size.
///
/// The person asked that the size line read exactly like the name: same
/// colour, same size, same font. Before this it was 10.5 px in the faint text
/// colour, a size down and a shade dimmer than the name above it, so it looked
/// like a different kind of thing. Both lines now come from here, so they
/// cannot drift apart again. None of these numbers is measured from the
/// original: Dolphin draws no size line under a thumbnail (the caption is an
/// added feature), and the name keeps the size this rebuild already used.
pub const CAPTION_SIZE: f32 = 12.0;

/// Baseline-to-baseline step of two caption lines, in px — one line plus its
/// leading, so name and size sit as evenly spaced as two lines of text.
pub const CAPTION_STEP: f32 = 17.0;

/// The caption font: the same proportional face the file names use.
pub fn caption_font() -> FontId {
    FontId::proportional(CAPTION_SIZE)
}

/// The colour a caption line is drawn in. A marked cell lifts it to white so
/// it reads on the selection band, exactly as the name does; everywhere else
/// it is the ordinary text colour.
pub fn caption_colour(selected: bool) -> Color32 {
    if selected {
        theme::on_band()
    } else {
        theme::text()
    }
}

/// Height the two caption lines take under a thumbnail. Both lines are the
/// same size now, so they take the same room.
fn caption_height(names: bool, dims: bool) -> f32 {
    let mut h = 0.0;
    if names {
        h += CAPTION_STEP;
    }
    if dims {
        h += CAPTION_STEP;
    }
    h
}

/// Room reserved under each picture for its name and its size line.
pub fn wall_caption(names: bool, dims: bool) -> f32 {
    caption_height(names, dims)
}

/// Narrowest and widest shape the packer will believe. A damaged header or a
/// 20:1 panorama must not swallow a whole row.
pub const ASPECT_MIN: f32 = 0.2;
pub const ASPECT_MAX: f32 = 6.0;

/// A picture is never packed shorter than this.
pub const ROW_MIN_PX: f32 = 24.0;

/// Width divided by height, held inside the range the packer trusts.
pub fn wall_aspect(aspect: f32) -> f32 {
    if !aspect.is_finite() || aspect <= 0.0 {
        1.0
    } else {
        aspect.clamp(ASPECT_MIN, ASPECT_MAX)
    }
}

/// One row of the wall: which pictures it holds and how tall they are. Every
/// row carries the height the person set, so `pic_h` says where the row sits on
/// the wall as well: rows are stacked `pic_h + caption + gap` apart.
pub struct WallRow {
    pub start: usize,
    pub end: usize,
    /// Height of the pictures themselves in this row.
    pub pic_h: f32,
}

/// Pack pictures into rows the width of the window, the way photo walls do.
///
/// Each picture is as wide as its own shape needs at the row's height, so the
/// frame around a thumbnail is the picture and nothing else — no square
/// footprint with a band of empty wall on both sides of a wide shot.
///
/// Every row is exactly as tall as the size the person set, the last row
/// included: the wall is a size, not an average. Rows are filled left to right
/// while the next picture still fits, and the row that runs out of room ends
/// there. (The earlier version stretched each row until it filled the width,
/// which made every row a slightly different height — the last, half-empty row
/// came out at the full 400 px while the rows above it sat at 380 or so.)
///
/// A picture wider than the whole wall at that size — a very long panorama —
/// is the one exception: alone in its row, it is drawn as wide as the wall, so
/// it ends up shorter than the rest.
///
/// `caption` is the room under each picture for its name and size line, `gap`
/// the space between neighbouring pictures and below a row.
pub fn wall_rows(
    aspects: &[f32],
    avail: f32,
    target: f32,
    gap: f32,
    caption: f32,
) -> (Vec<WallRow>, f32) {
    let avail = avail.max(80.0);
    let target = clamp_thumb(target);
    let mut rows: Vec<WallRow> = Vec::new();
    let mut y = 0.0f32;
    let mut i = 0usize;
    while i < aspects.len() {
        let mut pic_h = target;
        let mut count = 0usize;
        let mut used = 0.0f32;
        while i + count < aspects.len() {
            let a = wall_aspect(aspects[i + count]);
            let w = a * pic_h;
            if count == 0 {
                if w > avail {
                    // Alone in its row and wider than the wall: shrink this one
                    // row so the picture stays whole.
                    pic_h = (avail / a).max(ROW_MIN_PX);
                    used = avail;
                    count = 1;
                    break;
                }
                used = w;
                count = 1;
                continue;
            }
            let need = used + gap + w;
            if need > avail + 0.01 {
                break;
            }
            used = need;
            count += 1;
        }
        let _ = used;
        rows.push(WallRow {
            start: i,
            end: i + count,
            pic_h,
        });
        y += pic_h + caption + gap;
        i += count;
    }
    let total = if rows.is_empty() { 0.0 } else { (y - gap).max(0.0) };
    (rows, total)
}

/// Width of one picture in a row, from its own shape.
pub fn wall_pic_width(row: &WallRow, aspect: f32) -> f32 {
    (row.pic_h * wall_aspect(aspect)).max(6.0)
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
                    .color(theme::text_dim()),
            );
            let mut px = self.thumb_px;
            let slider = theme::strip_slider(ui, &mut px, THUMB_MIN..=THUMB_MAX, 132.0)
            .on_hover_text(
                "Height of each row of thumbnails — every picture keeps its own \
                 shape and the rows fill the window, so no empty band sits \
                 between them",
            );
            if slider.changed() {
                self.thumb_px = clamp_thumb(px);
                vctx.reqs.push(Request::GalleryChanged);
            }
            ui.label(
                egui::RichText::new(format!("{} px", self.thumb_px.round() as u32))
                    .size(11.0)
                    .color(theme::text_faint()),
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
            // Added feature: the wall can reach into the folders below, so a
            // photo library sorted into year or month folders shows as one wall.
            let mut below = self.gallery_below;
            if ui
                .add(egui::Checkbox::new(
                    &mut below,
                    egui::RichText::new("Sub-folders").size(11.5),
                ))
                .on_hover_text(
                    "Show the pictures in the folders below this one too — every \
                     folder on the way down, pictures only",
                )
                .changed()
            {
                self.set_pictures_below(below);
            }

            let pictures = self.gallery_images();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let play = egui::Button::new(egui::RichText::new("Slideshow").size(11.5))
                    .fill(theme::c(46, 53, 59));
                if ui
                    .add(play)
                    .on_hover_text("Play the pictures full screen, one every 1.5 s to start (Space)")
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
                        .fill(theme::c(46, 53, 59));
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
                let scanning = self
                    .search
                    .as_ref()
                    .map(|s| s.pictures_below && !s.done)
                    .unwrap_or(false);
                let count_text = if self.gallery_below {
                    if scanning {
                        format!("Looking… {} pictures so far", pictures.len())
                    } else {
                        format!(
                            "{} picture{} below",
                            pictures.len(),
                            if pictures.len() == 1 { "" } else { "s" }
                        )
                    }
                } else {
                    format!(
                        "{} picture{}",
                        pictures.len(),
                        if pictures.len() == 1 { "" } else { "s" }
                    )
                };
                ui.label(
                    egui::RichText::new(count_text)
                        .size(11.0)
                        .color(theme::text_faint()),
                );
            });
        });
    }

    /// The gallery itself: a grid of picture cards, in the pane's own sort
    /// order, with the pane's own selection rules.
    pub fn ui_pictures(&mut self, ui: &mut egui::Ui, vctx: &mut ViewCtx) {
        egui::Frame::new()
            .fill(theme::c(31, 34, 37))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                self.gallery_bar(ui, vctx);
            });

        // The wall holds the folder's pictures and nothing else. Folders used to
        // sit here as square tiles so the person could walk into the next folder
        // of photographs; the person asked for pictures alone, so a sub-folder or
        // a document is left to the grid and list views behind the wall, and the
        // path bar above the wall is how one walks into a sub-folder.
        let entries: Vec<(usize, Entry)> = self
            .shown_indices()
            .into_iter()
            .filter_map(|i| self.display_entries().get(i).map(|e| (i, e.clone())))
            .collect();
        if entries.is_empty() {
            let rect = ui.available_rect_before_wrap();
            let scanning = self
                .search
                .as_ref()
                .map(|s| s.pictures_below && !s.done)
                .unwrap_or(false);
            let (title, hint) = if scanning {
                (
                    "Looking through the folders below…",
                    "The pictures appear as they are found.",
                )
            } else if self.gallery_below {
                (
                    "No pictures in this folder or below it",
                    "Sub-folders is on; untick it to go back to this folder alone.",
                )
            } else {
                (
                    "No pictures in this folder",
                    "The wall shows pictures only; the list views show every file.",
                )
            };
            ui.painter().text(
                Pos2::new(rect.center().x, rect.top() + 90.0),
                Align2::CENTER_CENTER,
                title,
                FontId::proportional(14.0),
                theme::text_dim(),
            );
            ui.painter().text(
                Pos2::new(rect.center().x, rect.top() + 112.0),
                Align2::CENTER_CENTER,
                hint,
                FontId::proportional(12.0),
                theme::text_faint(),
            );
            return;
        }

        // Each picture is laid out at its own shape, so the wall needs the
        // width divided by height of every entry before it can pack the rows.
        // A picture whose header cannot be read is treated as square.
        let previews = self.previews;
        let aspects: Vec<f32> = entries
            .iter()
            .map(|(_, e)| {
                if previews {
                    wall_aspect(vctx.thumbs.aspect(&e.path))
                } else {
                    1.0
                }
            })
            .collect();

        let target = clamp_thumb(self.thumb_px);
        let caption = wall_caption(self.gallery_names, self.gallery_dims);
        let view_h = ui.available_height();
        let full = (ui.available_width() - GRID_PAD).max(120.0);
        let (mut rows, total) = wall_rows(&aspects, full, target, THUMB_GAP, caption);
        // A scroll bar takes its strip out of the inside width; pack again with
        // it removed so the last picture of a row is never pushed under the bar.
        if total > view_h {
            let narrowed = (full - 14.0).max(120.0);
            let packed = wall_rows(&aspects, narrowed, target, THUMB_GAP, caption);
            if packed.1 <= total + 0.5 {
                rows = packed.0;
            }
        }

        let selected_colour = if vctx.active || !vctx.split {
            theme::row_sel_active()
        } else {
            theme::row_sel_inactive()
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(GRID_PAD * 0.5);
                let clip = ui.clip_rect();
                for row in &rows {
                    let row_h = row.pic_h + caption + THUMB_GAP;
                    // Rows out of sight keep their place without being drawn or
                    // made clickable, so a folder of a thousand pictures costs
                    // the same as the dozen on screen.
                    let top = ui.cursor().top();
                    if top > clip.bottom() + 8.0
                        || top + row.pic_h + caption < clip.top() - 8.0
                    {
                        ui.add_space(row_h);
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(THUMB_GAP, 0.0);
                        ui.add_space(GRID_PAD * 0.5);
                        for k in row.start..row.end {
                            let (disp, entry) = (entries[k].0, entries[k].1.clone());
                            let pic_w = wall_pic_width(row, aspects[k]);
                            let card = Vec2::new(pic_w, row.pic_h + caption);
                            let (rect, resp) = ui.allocate_exact_size(card, Sense::click());
                            if self.scroll_to == Some(disp) {
                                ui.scroll_to_rect(rect, Some(Align::Center));
                            }
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let selected = self.sel.contains(&disp);
                            let bg = if selected {
                                selected_colour
                            } else if resp.hovered() {
                                theme::row_hover()
                            } else {
                                Color32::TRANSPARENT
                            };
                            if bg != Color32::TRANSPARENT {
                                ui.painter().rect_filled(
                                    rect.expand2(Vec2::new(2.0, 1.0)),
                                    CornerRadius::same(6),
                                    bg,
                                );
                            }
                            let behind =
                                if bg == Color32::TRANSPARENT { theme::view_bg() } else { bg };
                            self.draw_gallery_card(ui, rect, &entry, behind, vctx);
                            self.handle_row_interaction(ui, rect, &resp, disp, vctx);
                        }
                    });
                    ui.add_space(THUMB_GAP);
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
        card: Rect,
        entry: &Entry,
        behind: Color32,
        vctx: &mut ViewCtx,
    ) {
        // The frame is the picture's own shape: no square box, so no band of
        // empty wall beside a wide shot. `card` holds the picture on top and
        // the captions underneath.
        let caption = wall_caption(self.gallery_names, self.gallery_dims);
        let pic_rect = Rect::from_min_size(
            card.min,
            Vec2::new(card.width().max(6.0), (card.height() - caption).max(8.0)),
        );
        ui.painter().rect_filled(
            pic_rect,
            CornerRadius::same(if self.gallery_round { PICTURE_CORNER } else { 2 }),
            theme::c(24, 27, 30),
        );

        let mut drew = false;
        if !entry.is_dir && entry.has_preview() && self.previews {
            let px = (pic_rect.height().max(pic_rect.width()) * ui.ctx().pixels_per_point())
                .round() as u32;
            if let Some(tex) = vctx.thumbs.get(ui.ctx(), &entry.path, px.max(16)) {
                let src = tex.size();
                if src[0] > 0 && src[1] > 0 {
                    // The thumbnail keeps the picture's aspect, and so does the
                    // frame, so the picture fills it edge to edge.
                    ui.painter().image(
                        tex.id(),
                        pic_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    // "rounded image corner in thumbnail toggle": the picture
                    // takes the frame's rounding when the switch is on, and its
                    // own square corners when it is off.
                    if self.gallery_round {
                        round_picture(
                            ui.painter(),
                            pic_rect,
                            PICTURE_CORNER as f32,
                            behind,
                        );
                    }
                    drew = true;
                }
            }
        }
        if !drew {
            let side = (pic_rect.height().min(pic_rect.width()) * 0.45).clamp(24.0, 96.0);
            theme::draw_kind_icon_tinted(
                ui.painter(),
                Rect::from_center_size(pic_rect.center(), Vec2::splat(side)),
                if entry.is_symlink { Kind::Link } else { entry.kind },
                entry.broken,
                entry
                    .tint
                    .filter(|_| entry.is_dir)
                    .map(|t| theme::c(t[0], t[1], t[2])),
            );
        }
        // The browser frames each picture with a two-pixel border; the same
        // frame here tells a very light picture apart from the dark grid. With
        // rounding switched off the frame keeps square corners to match.
        ui.painter().rect_stroke(
            pic_rect,
            CornerRadius::same(if self.gallery_round { PICTURE_CORNER } else { 2 }),
            Stroke::new(2.0, theme::c(58, 64, 70)),
            egui::StrokeKind::Inside,
        );

        let mut y = pic_rect.bottom() + 4.0;
        if self.gallery_names {
            let max_w = (card.width() - 4.0).max(18.0);
            let font = caption_font();
            let name = elide(ui, &entry.name, max_w, font.clone());
            ui.painter().text(
                Pos2::new(card.center().x, y + 7.0),
                Align2::CENTER_CENTER,
                name,
                font,
                theme::text(),
            );
            y += CAPTION_STEP;
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
                // The size line is a caption like the name above it: same
                // font, same size, same colour, one caption step below.
                let max_w = (card.width() - 4.0).max(18.0);
                let font = caption_font();
                ui.painter().text(
                    Pos2::new(card.center().x, y + 7.0),
                    Align2::CENTER_CENTER,
                    elide(ui, &caption, max_w, font.clone()),
                    font,
                    caption_colour(false),
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

/// What drawing the prompt block produced: the message a button wants shown in
/// the status line, and where the selectable prompt text was drawn (the
/// self-check drags across it to prove the text really can be selected).
#[derive(Default)]
pub struct PromptUi {
    pub message: Option<String>,
    pub field: Option<egui::Rect>,
    /// The visible part of the prompt box, which is shorter than `field` when
    /// the prompt is long enough to scroll.
    pub view: Option<egui::Rect>,
    /// The selection the person made, as character indices into the prompt.
    pub selection: Option<(usize, usize)>,
    /// How far the prompt box is scrolled, in pixels.
    pub scroll: f32,
    /// True when the block was folded away this frame: no fields, no buttons and
    /// no text were drawn, only the title row and one faint line of the prompt.
    pub collapsed: bool,
    /// Where the title row was drawn, so a check can click it.
    pub header: Option<egui::Rect>,
}

/// How tall the block's title row is — the row the person clicks to fold the
/// prompt away and to bring it back.
pub const PROMPT_HEADER_H: f32 = 22.0;

/// The title row of the prompt block: a fold arrow that points down while the
/// block is open and right while it is folded, the word "Prompt", and the name
/// of the tool that wrote the recipe on the right. The whole row is one hit
/// target, so a click anywhere on it folds or unfolds the block.
///
/// Returns true when the person clicked it, and where the row was drawn.
fn prompt_header(ui: &mut egui::Ui, open: bool, source: Option<&str>) -> (bool, egui::Rect) {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), PROMPT_HEADER_H),
        Sense::click(),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), theme::header_bg());
    }
    // The arrow: a small solid triangle, filled while open and pointing right
    // while folded, the way a fold arrow reads everywhere else in this build.
    let c = Pos2::new(rect.left() + 9.0, rect.center().y);
    let s = 4.0;
    let arrow = if open {
        vec![
            Pos2::new(c.x - s, c.y - s * 0.55),
            Pos2::new(c.x + s, c.y - s * 0.55),
            Pos2::new(c.x, c.y + s * 0.75),
        ]
    } else {
        vec![
            Pos2::new(c.x - s * 0.55, c.y - s),
            Pos2::new(c.x + s * 0.75, c.y),
            Pos2::new(c.x - s * 0.55, c.y + s),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(arrow, theme::text_dim(), Stroke::NONE));
    ui.painter().text(
        Pos2::new(rect.left() + 22.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Prompt",
        FontId::proportional(12.0),
        theme::text(),
    );
    if let Some(src) = source {
        ui.painter().text(
            Pos2::new(rect.right() - 2.0, rect.center().y),
            Align2::RIGHT_CENTER,
            src,
            FontId::proportional(9.5),
            theme::text_faint(),
        );
    }
    let hit = resp
        .on_hover_text(if open {
            "Fold the prompt away — it is not needed all the time"
        } else {
            "Show the prompt"
        })
        .clicked();
    (hit, rect)
}

/// The prompt block shared by the viewer's side panel and the information panel:
/// the fields first (Steps, Sampler, Seed…), then the prompt itself in a
/// read-only box whose text can be selected with the mouse and copied with
/// Ctrl+C, then the negative prompt, then where it came from.
///
/// Added feature: the whole block folds away behind its title row, and `open`
/// is the person's own choice, kept between runs. While folded only the title
/// row and one faint line of the prompt are drawn, so the picture and the rest
/// of the panel get the room back.
pub fn prompt_panel(
    ui: &mut egui::Ui,
    meta: Option<&crate::imagemeta::ImageMeta>,
    picture: Option<&Path>,
    open: &mut bool,
) -> PromptUi {
    let mut out = PromptUi::default();
    let source = meta.map(|m| m.source.clone());
    let (clicked, header_rect) = prompt_header(ui, *open, source.as_deref());
    out.header = Some(header_rect);
    if clicked {
        *open = !*open;
        out.message = Some(if *open {
            "Prompt shown.".to_string()
        } else {
            "Prompt folded away.".to_string()
        });
    }
    out.collapsed = !*open;
    if !*open {
        // One faint line of the prompt stays behind, so a folded block still
        // says which picture it belongs to without taking the panel over.
        if let Some(m) = meta {
            let first = m.prompt_only().lines().next().unwrap_or("").trim().to_string();
            if !first.is_empty() {
                let w = (ui.available_width() - 24.0).max(40.0);
                ui.label(
                    egui::RichText::new(elide(ui, &first, w, FontId::proportional(10.5)))
                        .size(10.5)
                        .color(theme::text_faint()),
                );
            }
        }
        ui.add_space(2.0);
        return out;
    }
    ui.add_space(2.0);
    let Some(m) = meta else {
        ui.label(
            egui::RichText::new("No generation prompt is stored in this picture.")
                .size(11.5)
                .color(theme::text_dim()),
        );
        ui.label(
            egui::RichText::new("Pictures made by common text-to-image tools carry theirs; photos usually do not.")
                .size(10.5)
                .color(theme::text_faint()),
        );
        return out;
    };

    // The prompt text is needed by the buttons (Select all needs its length)
    // and by the block itself, so it is gathered once.
    let text = m.prompt_only();

    // Four ways out of the block: the whole prompt selected, the prompt alone,
    // the whole recipe, or a text file beside the picture.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui
            .add(egui::Button::new("Select all").small())
            .on_hover_text("Select the whole prompt, so it can be read through before copying")
            .clicked()
        {
            crate::selectable::select_all(
                ui.ctx(),
                egui::Id::new("prompt-selectable-text"),
                text.chars().count(),
            );
            out.message = Some("The whole prompt is selected — Ctrl+C copies it.".to_string());
        }
        if ui
            .add(egui::Button::new("Copy prompt").small())
            .on_hover_text("Copy the prompt on its own")
            .clicked()
        {
            ui.ctx().copy_text(text.clone());
            out.message = Some("Prompt copied to the clipboard.".to_string());
        }
        if ui
            .add(egui::Button::new("Copy all").small())
            .on_hover_text("Copy the prompt, the negative prompt and the settings")
            .clicked()
        {
            ui.ctx().copy_text(m.recipe_text());
            out.message = Some("Prompt, negative prompt and settings copied.".to_string());
        }
        if ui
            .add(egui::Button::new("Save").small())
            .on_hover_text("Save the prompt in a text file next to the picture")
            .clicked()
        {
            match picture {
                Some(p) => match m.save_beside(p) {
                    Ok(written) => {
                        out.message = Some(format!(
                            "Saved {} beside the picture.",
                            written
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "the prompt".into())
                        ))
                    }
                    Err(e) => out.message = Some(format!("Could not save the prompt: {e}")),
                },
                None => out.message = Some("Saving needs a picture on disk.".to_string()),
            }
        }
    });

    for (k, v) in &m.fields {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{k}:")).size(10.5).color(theme::text_dim()));
            ui.label(
                egui::RichText::new(elide(ui, v, 190.0, FontId::proportional(11.0)))
                    .size(11.0)
                    .strong()
                    .color(theme::text()),
            );
        });
    }
    if !m.fields.is_empty() {
        ui.add_space(3.0);
    }

    // The prompt sits in a block that lays the whole text out and scrolls it
    // itself, so a drag that runs past the bottom of the box keeps scrolling
    // and keeps selecting — the person asked for selection to take scrolling
    // into account. A text field clipped inside a scroll area could only ever
    // select the part that happened to be in view.
    let block = crate::selectable::text_block(
        ui,
        egui::Id::new("prompt-selectable-text"),
        &text,
        150.0,
    );
    out.field = Some(block.rect);
    out.view = Some(block.view);
    out.selection = block.selection;
    out.scroll = block.scroll;
    ui.label(
        egui::RichText::new(
            "Drag to select — the box scrolls when the drag reaches its edge. Ctrl+A selects all, Ctrl+C copies, Shift+click extends.",
        )
        .size(9.5)
        .color(theme::text_faint()),
    );

    if let Some(neg) = &m.negative {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Negative prompt")
                .size(10.0)
                .color(theme::text_faint()),
        );
        crate::selectable::text_block(
            ui,
            egui::Id::new("negative-prompt-selectable-text"),
            neg,
            96.0,
        );
    }
    out
}

/// Shorten a name to `max_w` with an ellipsis, measured with the same font it
/// is drawn in.
fn elide(ui: &egui::Ui, text: &str, max_w: f32, font: FontId) -> String {
    let painter = ui.painter();
    let full = painter.layout_no_wrap(text.to_owned(), font.clone(), theme::text());
    if full.size().x <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut trial = out.clone();
        trial.push(ch);
        trial.push('…');
        let w = painter
            .layout_no_wrap(trial.clone(), font.clone(), theme::text())
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

/// Whether the two pictures share one framing. In Split and Overlay the pair is
/// drawn in one frame, so they are always locked together; Side mode is the only
/// one with a Sync switch of its own — the browser's `isCompareSynced` returns
/// true for split and overlay whatever that switch says.
pub fn compare_synced(mode: CompareMode, sync: bool) -> bool {
    !matches!(mode, CompareMode::Side) || sync
}

/// One wheel notch reaches the window as two line events, a press and a
/// release, a few milliseconds apart. Without a guard each notch was counted
/// twice: two notches of a real wheel ran the zoom to 146 % instead of 121 %.
pub const WHEEL_DUPLICATE_S: f64 = 0.02;

/// A picture's wheel state: the remainder of a partial notch, and when the last
/// notch was counted, so the second half of a notch is not counted again.
#[derive(Default, Clone, Copy)]
pub struct Wheel {
    acc: f32,
    last_time: f64,
    last_dir: f32,
}

impl Wheel {
    pub fn reset(&mut self) {
        self.acc = 0.0;
        self.last_time = 0.0;
        self.last_dir = 0.0;
    }

    /// How many zoom steps this frame's wheel events are worth.
    pub fn steps(&mut self, time: f64, units: f32) -> i32 {
        if units.abs() < 1.0 {
            return 0;
        }
        let dir = if units > 0.0 { 1.0 } else { -1.0 };
        let repeat = self.last_dir == dir && (time - self.last_time) < WHEEL_DUPLICATE_S;
        self.last_time = time;
        self.last_dir = dir;
        if repeat {
            return 0;
        }
        crate::state::wheel_zoom_steps(&mut self.acc, units)
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
    /// The right-hand picture's own wheel state while Sync is off.
    pub wheel_b: Wheel,
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

    /// A show that starts the way the person last left it: their interval
    /// (1.5 s until they change it) and their fade switch.
    pub fn remembered(interval: f32, fade: bool) -> Self {
        let mut slide = Self::new(interval);
        slide.fade = fade;
        slide
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
    /// Wheel state: the remainder of a partial notch and the notch guard.
    pub wheel: Wheel,
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
            wheel: Wheel::default(),
            show_info: false,
            slide: None,
            compare: None,
            shown_at: Instant::now(),
            show_help: false,
            fullscreen: play,
            fullscreen_done: false,
            meta: None,
        };
        if play {
            v.slide = Some(Slide::new(SLIDE_DEFAULT));
        }
        v
    }

    /// Read the recipe of the picture on screen, once per picture rather than
    /// once per frame.
    ///
    /// This deliberately leaves `show_info` alone. The details panel used to
    /// open itself the first time a picture carrying a prompt came up; the
    /// person asked for it to be closed by default, so the panel now only ever
    /// opens from the info button in the bar, the I key, or the Pictures menu.
    pub fn sync_meta(&mut self) {
        if let Some(path) = self.current().cloned() {
            if !matches!(&self.meta, Some((p, _)) if *p == path) {
                self.meta = Some((path.clone(), crate::imagemeta::read(&path)));
            }
        }
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
        self.wheel.reset();
        self.shown_at = Instant::now();
        if let Some(slide) = self.slide.as_mut() {
            slide.next_at = Instant::now() + Duration::from_secs_f32(slide.interval);
        }
        // Paging while a comparison is open moves the pair with the folder, so
        // the names under the pictures keep naming the pictures above them.
        if self.compare.is_some() {
            let a = self.current().cloned().unwrap_or_default();
            let b = self
                .items
                .get((self.index + 1) % n.max(1) as usize)
                .cloned()
                .unwrap_or_else(|| a.clone());
            if let Some(cmp) = self.compare.as_mut() {
                cmp.a = a;
                cmp.b = b;
                cmp.zoom_b = 1.0;
                cmp.pan_b = Vec2::ZERO;
                cmp.wheel_b.reset();
            }
        }
    }

    /// One zoom step from the buttons or the keyboard. While the pair is synced
    /// the partner keeps the same framing, so turning Sync off later starts from
    /// where the two already are.
    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = zoom_step(self.zoom, factor);
        self.fit = false;
        let (z, p) = (self.zoom, self.pan);
        if let Some(cmp) = self.compare.as_mut() {
            if compare_synced(cmp.mode, cmp.sync) {
                cmp.zoom_b = z;
                cmp.pan_b = p;
            }
        }
    }

    /// Back to 100 % and the middle of the pane — both pictures when a
    /// comparison is open.
    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.fit = true;
        self.wheel.reset();
        if let Some(cmp) = self.compare.as_mut() {
            cmp.zoom_b = 1.0;
            cmp.pan_b = Vec2::ZERO;
            cmp.wheel_b.reset();
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
            wheel_b: Wheel::default(),
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
            .layout_no_wrap(label.to_owned(), font.clone(), theme::text())
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
        theme::on_band()
    } else if resp.hovered() {
        theme::text()
    } else {
        theme::text_dim()
    };
    if active || resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(3),
            if active {
                theme::c(52, 60, 68)
            } else {
                theme::c(40, 46, 52)
            },
        );
    }
    if active {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, theme::accent()),
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
        // Space starts a show at the interval and fade the person last set, so
        // the 1.5 s default is a starting point rather than a reset.
        let saved_slide = clamp_slide(self.config.slide_secs);
        let saved_fade = self.config.slide_fade;
        let mut close = false;
        let mut needs_two = false;
        let mut fullscreen = false;
        // Added feature: P folds the prompt block in the viewer's side panel
        // away and brings it back, the same switch its title row offers.
        let mut want_prompt = false;
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
                        viewer.slide = Some(Slide::remembered(saved_slide, saved_fade));
                    }
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Plus)
                    || i.consume_key(egui::Modifiers::NONE, Key::Equals)
                {
                    viewer.zoom_by(ZOOM_BUTTON_STEP);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Minus) {
                    viewer.zoom_by(1.0 / ZOOM_BUTTON_STEP);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Num0) {
                    viewer.reset_zoom();
                }
                if i.consume_key(egui::Modifiers::NONE, Key::C) && !viewer.start_compare() {
                    needs_two = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::I) {
                    viewer.show_info = !viewer.show_info;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::P) {
                    // P is "show me the prompt": with the details panel closed
                    // by default the first press has to open it, or the key
                    // would do nothing at all on a fresh picture.
                    if viewer.show_info {
                        want_prompt = true;
                    } else {
                        viewer.show_info = true;
                    }
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
        if want_prompt {
            self.config.prompt_open = !self.config.prompt_open;
            self.config_dirty = true;
            let word = if self.config.prompt_open { "shown" } else { "folded away" };
            self.toast(&format!("Prompt {word}."));
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
        // Starting a show from the bar uses the person's saved interval and
        // fade, the same ones the Space key and a fresh window use.
        let saved_slide = clamp_slide(self.config.slide_secs);
        let saved_fade = self.config.slide_fade;
        let screen = ctx.screen_rect();
        let mut close = false;
        let mut open_compare_with_next = false;
        let mut want_compare_close = false;
        let mut want_swap = false;
        let mut wanted_mode: Option<CompareMode> = None;
        let mut prompt_message: Option<String> = None;
        // Added feature: the prompt block folds away, and the choice is kept
        // between runs. Taken out of the config here so the viewer's own borrow
        // below stays clean, and put back when the frame is over.
        let mut prompt_open = self.config.prompt_open;

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

        // "extract meta data from prompt": the picture's own recipe is read when
        // it comes up, but the details panel that shows it stays closed until
        // the person asks for it.
        if let Some(v) = self.viewer.as_mut() {
            v.sync_meta();
        }

        egui::Area::new(Id::new("picture_viewer"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.painter()
                    .rect_filled(screen, 0.0, theme::c(10, 12, 14));

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
                    .rect_filled(canvas, 0.0, theme::c(14, 16, 18));

                let viewer = self.viewer.as_mut().unwrap();
                let path = viewer.current().cloned().unwrap_or_default();
                let tex = self.big.get(ctx, &path);

                // ---- picture area ----------------------------------------
                // A comparison handles its own pointer: each picture has to be
                // zoomed and panned on its own (or together while synced), so a
                // single drag area over the whole canvas would be wrong.
                let comparing = viewer.compare.is_some();
                if !comparing {
                    let drag = ui.interact(canvas, Id::new("viewer_canvas"), Sense::click_and_drag());
                    if drag.dragged() {
                        viewer.pan += drag.drag_delta();
                    }
                    if drag.double_clicked() {
                        if viewer.zoom > 1.01 {
                            viewer.reset_zoom();
                        } else {
                            viewer.zoom = 2.0;
                            viewer.pan = Vec2::ZERO;
                            viewer.fit = false;
                        }
                    }
                    if drag.hovered() {
                        // One notch is one step: the wheel is read as events, not
                        // as a smoothed delta that lasts a dozen frames.
                        let units = wheel_units_in_frame(ui);
                        let time = ui.input(|i| i.time);
                        let steps = viewer.wheel.steps(time, units);
                        if steps != 0 {
                            let factor = if steps > 0 {
                                ZOOM_WHEEL_IN.powi(steps.min(4))
                            } else {
                                ZOOM_WHEEL_OUT.powi((-steps).min(4))
                            };
                            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                                let (z, p) = zoom_about(
                                    viewer.zoom,
                                    viewer.pan,
                                    pos - canvas.center(),
                                    factor,
                                );
                                viewer.zoom = z;
                                viewer.pan = p;
                            }
                            viewer.fit = false;
                        }
                    }
                }

                if comparing {
                    let Viewer {
                        zoom,
                        pan,
                        wheel,
                        compare,
                        ..
                    } = viewer;
                    draw_compare(
                        ui,
                        canvas,
                        zoom,
                        pan,
                        wheel,
                        compare.as_mut().unwrap(),
                        &mut self.big,
                        ctx,
                    );
                } else if let Some(tex) = tex.clone() {
                    let src = tex.size();
                    if src[0] > 0 && src[1] > 0 {
                        {
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
                    // A picture the decoder refused should not look like one
                    // that is still arriving: say which it is.
                    let refused = viewer
                        .current()
                        .map(|p| self.big.has_failed(p))
                        .unwrap_or(false);
                    let (note, colour) = if refused {
                        (
                            "This file could not be opened as a picture",
                            theme::c(220, 170, 140),
                        )
                    } else {
                        ("Loading picture…", theme::text_dim())
                    };
                    ui.painter().text(
                        canvas.center(),
                        Align2::CENTER_CENTER,
                        note,
                        FontId::proportional(14.0),
                        colour,
                    );
                }

                // ---- top bar ---------------------------------------------
                ui.painter()
                    .rect_filled(top, 0.0, theme::c(20, 23, 26));
                ui.painter().line_segment(
                    [Pos2::new(top.left(), top.bottom()), Pos2::new(top.right(), top.bottom())],
                    Stroke::new(1.0, theme::c(38, 43, 48)),
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
                        ui.label(egui::RichText::new(shown).size(13.0).color(theme::text()));
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                viewer.index + 1,
                                viewer.items.len()
                            ))
                            .size(11.0)
                            .color(theme::text_faint()),
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
                                    viewer.slide =
                                        Some(Slide::remembered(saved_slide, saved_fade));
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
                                        .color(theme::text_faint()),
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
                            if bar_button(ui, "plus", "", false)
                                .on_hover_text("Zoom in")
                                .clicked()
                            {
                                viewer.zoom_by(ZOOM_BUTTON_STEP);
                            }
                            // With Sync off in Side mode the two pictures hold
                            // their own zoom, so the read-out names both, the way
                            // the browser's `updateCompareZoomLabel` does.
                            let pct_a = (viewer.zoom * 100.0).round() as u32;
                            let zoom_text = match viewer.compare.as_ref() {
                                Some(c) if !compare_synced(c.mode, c.sync) => format!(
                                    "{pct_a}% / {}%",
                                    (c.zoom_b * 100.0).round() as u32
                                ),
                                _ => format!("{pct_a}%"),
                            };
                            ui.label(
                                egui::RichText::new(zoom_text)
                                    .size(11.0)
                                    .color(theme::text_dim()),
                            );
                            if bar_button(ui, "minus", "", false)
                                .on_hover_text("Zoom out")
                                .clicked()
                            {
                                viewer.zoom_by(1.0 / ZOOM_BUTTON_STEP);
                            }
                            if bar_button(ui, "fit", "Fit", viewer.fit)
                                .on_hover_text("Fit to window (0)")
                                .clicked()
                            {
                                viewer.reset_zoom();
                            }
                        });
                    },
                );

                // ---- bottom bar ------------------------------------------
                ui.painter()
                    .rect_filled(bottom, 0.0, theme::c(20, 23, 26));
                ui.painter().line_segment(
                    [
                        Pos2::new(bottom.left(), bottom.top()),
                        Pos2::new(bottom.right(), bottom.top()),
                    ],
                    Stroke::new(1.0, theme::c(38, 43, 48)),
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
                                    .color(theme::text_dim()),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(caption)
                                    .size(11.5)
                                    .color(theme::text_faint()),
                            );
                        }

                        if let Some(cmp) = viewer.compare.as_mut() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                if bar_button(ui, "swap", "Swap", false)
                                    .on_hover_text("Swap the two pictures")
                                    .clicked()
                                {
                                    want_swap = true;
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
                                        cmp.pan_b = viewer.pan;
                                    }
                                }
                                for mode in [
                                    CompareMode::Overlay,
                                    CompareMode::Split,
                                    CompareMode::Side,
                                ] {
                                    if bar_button(ui, mode.glyph(), mode.label(), cmp.mode == mode)
                                        .clicked()
                                        && cmp.mode != mode
                                    {
                                        cmp.mode = mode;
                                        wanted_mode = Some(mode);
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
                                            .color(theme::text_faint()),
                                    );
                                    ui.label(
                                        egui::RichText::new("Opacity")
                                            .size(11.0)
                                            .color(theme::text_dim()),
                                    );
                                }
                            });
                        }
                    },
                );

                if viewer.show_help {
                    let help = "Esc close   ← → page   Space slideshow   + − zoom   0 fit   I details   P prompt   F full screen   C compare";
                    ui.painter().text(
                        Pos2::new(canvas.center().x, canvas.top() + 18.0),
                        Align2::CENTER_CENTER,
                        help,
                        FontId::proportional(11.5),
                        theme::text_faint(),
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
                    // The viewer's control panel is a palette surface, not a black
                    // slab: its labels are drawn in the palette's text colour, which
                    // is light in dark mode and dark in light mode.
                    let panel_bg = theme::c(22, 26, 30);
                    ui.painter().rect_filled(
                        panel,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(
                            panel_bg.r(),
                            panel_bg.g(),
                            panel_bg.b(),
                            236,
                        ),
                    );
                    ui.painter().rect_stroke(
                        panel,
                        CornerRadius::same(6),
                        Stroke::new(1.0, theme::separator()),
                        egui::StrokeKind::Inside,
                    );
                    let inner = panel.shrink(10.0);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.set_clip_rect(inner);
                            let shown = prompt_panel(
                                ui,
                                viewer.meta.as_ref().and_then(|(_, m)| m.as_ref()),
                                viewer.meta.as_ref().map(|(p, _)| p.as_path()),
                                &mut prompt_open,
                            );
                            if let Some(msg) = shown.message {
                                prompt_message = Some(msg);
                            }
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
        // Swapping the pair is a new comparison, so both pictures start at 100 %
        // in the middle — the browser's `swapCompareImages` resets its zoom too.
        if want_swap {
            if let Some(v) = self.viewer.as_mut() {
                if let Some(c) = v.compare.as_mut() {
                    if c.a != c.b {
                        std::mem::swap(&mut c.a, &mut c.b);
                    }
                }
                v.reset_zoom();
            }
        }
        // A different mode frames the pair differently (one pane each, one split
        // canvas, or two stacked), so the zoom starts again from Fit.
        if let (Some(mode), Some(v)) = (wanted_mode, self.viewer.as_mut()) {
            if let Some(c) = v.compare.as_mut() {
                c.mode = mode;
            }
            v.reset_zoom();
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
        if let Some(msg) = prompt_message {
            self.toast(&msg);
        }
        // The fold arrow (or P) belongs to the person, so the state goes back
        // into the settings and is written out with the rest of them.
        if prompt_open != self.config.prompt_open {
            self.config.prompt_open = prompt_open;
            self.config_dirty = true;
        }
        if let Some(v) = self.viewer.as_ref() {
            if v.slide.is_some() {
                ctx.request_repaint_after(Duration::from_millis(50));
                let _ = slide_interval;
            }
        }
    }
}

/// Draw two pictures side by side, split at a divider, or one over the other,
/// and let the pointer act on them.
///
/// The framing rules, each one a fix for something the person hit:
///
/// * **A picture never leaves its pane.** Both pictures are clipped to the pane
///   they belong to; before this a zoomed right-hand picture was painted across
///   the whole canvas and hid its neighbour.
/// * **The point under the pointer stays put.** Zoom is anchored on the pane the
///   pointer is over, not on the middle of the window, so the detail being looked
///   at does not slide sideways as the wheel turns.
/// * **A drag only moves the picture that was grabbed**, and stops at the edges
///   (`clamp_pan`), so a picture can never be thrown out of sight.
/// * **Sync locks the pair.** With Sync on — and always in Split and Overlay,
///   which are one frame — the wheel and the drag move both pictures together.
/// * **One wheel notch is one step.** The wheel is read as events; the smoothed
///   delta it used to read is spread over a dozen frames.
#[allow(clippy::too_many_arguments)]
fn draw_compare(
    ui: &mut egui::Ui,
    canvas: Rect,
    zoom_a: &mut f32,
    pan_a: &mut Vec2,
    wheel_a: &mut Wheel,
    cmp: &mut Compare,
    big: &mut BigCache,
    ctx: &egui::Context,
) {
    let synced = compare_synced(cmp.mode, cmp.sync);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let units = wheel_units_in_frame(ui);

    // While the pair is locked together the partner's own framing is kept equal
    // to the first picture's, so turning Sync off starts where both already are.
    if synced {
        cmp.zoom_b = *zoom_a;
        cmp.pan_b = *pan_a;
    }

    let wheel_factor = |steps: i32| {
        if steps > 0 {
            ZOOM_WHEEL_IN.powi(steps.min(4))
        } else {
            ZOOM_WHEEL_OUT.powi((-steps).min(4))
        }
    };

    match cmp.mode {
        CompareMode::Side => {
            let gap = 8.0;
            let half = (canvas.width() - gap) * 0.5;
            let left = Rect::from_min_size(canvas.min, Vec2::new(half, canvas.height()));
            let right = Rect::from_min_size(
                Pos2::new(canvas.left() + half + gap, canvas.top()),
                Vec2::new(half, canvas.height()),
            );
            let tex_a = big.get(ctx, &cmp.a);
            let tex_b = big.get(ctx, &cmp.b);
            let src_a = tex_a.as_ref().map(|t| t.size()).unwrap_or([0, 0]);

            // One drag area per picture, so a drag is always the picture that
            // was under the pointer when the button went down.
            let resp_a = ui.interact(left, Id::new("cmp_pane_a"), Sense::click_and_drag());
            let resp_b = ui.interact(right, Id::new("cmp_pane_b"), Sense::click_and_drag());
            let over_b = pointer.map_or(false, |p| right.contains(p));

            // Wheel: zoom the picture under the pointer, or both while synced.
            if units.abs() > 1.0 {
                let time = ui.input(|i| i.time);
                let steps = if over_b && !synced {
                    cmp.wheel_b.steps(time, units)
                } else {
                    wheel_a.steps(time, units)
                };
                if steps != 0 {
                    if let Some(pos) = pointer {
                        let factor = wheel_factor(steps);
                        if over_b && !synced {
                            let (z, p) = zoom_about(cmp.zoom_b, cmp.pan_b, pos - right.center(), factor);
                            cmp.zoom_b = z;
                            cmp.pan_b = p;
                        } else {
                            let pane = if over_b { right } else { left };
                            let (z, p) = zoom_about(*zoom_a, *pan_a, pos - pane.center(), factor);
                            *zoom_a = z;
                            *pan_a = p;
                        }
                    }
                }
            }

            // Drag: the grabbed picture, or both while synced.
            let drag_a = resp_a.drag_delta();
            let drag_b = resp_b.drag_delta();
            if resp_a.dragged() || resp_b.dragged() {
                if synced {
                    *pan_a += drag_a + drag_b;
                } else if resp_a.dragged() {
                    *pan_a += drag_a;
                } else {
                    cmp.pan_b += drag_b;
                }
            }
            if resp_a.double_clicked() || resp_b.double_clicked() {
                let target_b = resp_b.double_clicked() && !synced;
                let (z, p): (&mut f32, &mut Vec2) = if target_b {
                    (&mut cmp.zoom_b, &mut cmp.pan_b)
                } else {
                    (zoom_a, pan_a)
                };
                if *z > 1.01 {
                    *z = 1.0;
                    *p = Vec2::ZERO;
                } else {
                    *z = 2.0;
                    *p = Vec2::ZERO;
                }
            }

            // Clamp before drawing: a picture may be pushed to its edges, never
            // past them.
            let pane = left.size();
            let size_a = fitted_size(src_a, *zoom_a, pane);
            *pan_a = clamp_pan(*pan_a, size_a, pane);
            if !synced {
                if let Some(tb) = tex_b.as_ref() {
                    let size_b = fitted_size(tb.size(), cmp.zoom_b, pane);
                    cmp.pan_b = clamp_pan(cmp.pan_b, size_b, pane);
                }
            } else {
                cmp.pan_b = *pan_a;
            }

            match tex_a.as_ref() {
                Some(ta) => draw_fit(ui, left, ta, *zoom_a, *pan_a),
                None => loading(ui, left),
            }
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(left.right(), canvas.top()),
                    Vec2::new(gap, canvas.height()),
                ),
                0.0,
                theme::c(30, 34, 38),
            );
            match tex_b.as_ref() {
                Some(tb) => draw_fit(ui, right, tb, cmp.zoom_b, cmp.pan_b),
                None => loading(ui, right),
            }

            // A dark strip behind each name, otherwise the name is written on
            // top of the photograph and cannot be read.
            let painter = ui.painter();
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
                    theme::on_scrim(),
                );
            }
        }
        CompareMode::Split => {
            // The two pictures share one frame, so they are always synced: the
            // divider only decides which one is visible on each side.
            let mut x = canvas.left() + canvas.width() * cmp.split_at;
            let band = 10.0;
            let left_zone = Rect::from_min_max(canvas.min, Pos2::new(x - band, canvas.bottom()));
            let right_zone = Rect::from_min_max(Pos2::new(x + band, canvas.top()), canvas.max);
            let resp_l = ui.interact(left_zone, Id::new("cmp_split_l"), Sense::click_and_drag());
            let resp_r = ui.interact(right_zone, Id::new("cmp_split_r"), Sense::click_and_drag());
            // The divider is registered last, so it sits above the two drag
            // areas and its own drag never pans the pictures.
            let handle = ui.interact(
                Rect::from_min_size(
                    Pos2::new(x - band, canvas.top()),
                    Vec2::new(band * 2.0, canvas.height()),
                ),
                Id::new("compare_divider"),
                Sense::drag(),
            );
            if handle.dragged() {
                if let Some(pos) = handle.interact_pointer_pos() {
                    cmp.split_at = ((pos.x - canvas.left()) / canvas.width()).clamp(0.05, 0.95);
                    x = canvas.left() + canvas.width() * cmp.split_at;
                }
            }

            if units.abs() > 1.0 {
                let time = ui.input(|i| i.time);
                let steps = wheel_a.steps(time, units);
                if steps != 0 {
                    if let Some(pos) = pointer {
                        let (z, p) =
                            zoom_about(*zoom_a, *pan_a, pos - canvas.center(), wheel_factor(steps));
                        *zoom_a = z;
                        *pan_a = p;
                    }
                }
            }
            let drag = resp_l.drag_delta() + resp_r.drag_delta();
            if resp_l.dragged() || resp_r.dragged() {
                *pan_a += drag;
            }
            if resp_l.double_clicked() || resp_r.double_clicked() {
                if *zoom_a > 1.01 {
                    *zoom_a = 1.0;
                    *pan_a = Vec2::ZERO;
                } else {
                    *zoom_a = 2.0;
                    *pan_a = Vec2::ZERO;
                }
            }

            let tex_a = big.get(ctx, &cmp.a);
            let tex_b = big.get(ctx, &cmp.b);
            if let Some(ta) = tex_a.as_ref() {
                let size = fitted_size(ta.size(), *zoom_a, canvas.size());
                *pan_a = clamp_pan(*pan_a, size, canvas.size());
            }
            match tex_a.as_ref() {
                Some(ta) => {
                    let half = Rect::from_min_max(canvas.min, Pos2::new(x, canvas.bottom()));
                    draw_fit_clipped(ui, canvas, half, ta, *zoom_a, *pan_a);
                }
                None => loading(ui, canvas),
            }
            if let Some(tb) = tex_b.as_ref() {
                let half = Rect::from_min_max(Pos2::new(x, canvas.top()), canvas.max);
                draw_fit_clipped(ui, canvas, half, tb, *zoom_a, *pan_a);
            }
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(x - 1.0, canvas.top()),
                    Vec2::new(2.0, canvas.height()),
                ),
                0.0,
                theme::c(220, 226, 232),
            );
            // A grip, so the divider can be found with the pointer.
            ui.painter().circle_filled(
                Pos2::new(x, canvas.center().y),
                9.0,
                theme::c(220, 226, 232),
            );
            // Each name sits on a dark tab beside the divider: written straight
            // onto a photograph it could not be read.
            for (name, above) in [(file_label(&cmp.a), true), (file_label(&cmp.b), false)] {
                let font = FontId::proportional(10.5);
                let w = ui
                    .painter()
                    .layout_no_wrap(name.clone(), font.clone(), theme::text())
                    .size()
                    .x;
                let y = if above {
                    canvas.center().y - 30.0
                } else {
                    canvas.center().y + 30.0
                };
                let tab = Rect::from_min_size(Pos2::new(x - w - 14.0, y - 9.0), Vec2::new(w + 12.0, 18.0));
                ui.painter()
                    .rect_filled(tab, CornerRadius::same(3), Color32::from_black_alpha(170));
                ui.painter().text(
                    Pos2::new(tab.left() + 6.0, y),
                    Align2::LEFT_CENTER,
                    name,
                    font,
                    theme::on_scrim(),
                );
            }
        }
        CompareMode::Overlay => {
            let tex_a = big.get(ctx, &cmp.a);
            let tex_b = big.get(ctx, &cmp.b);
            let resp = ui.interact(canvas, Id::new("cmp_overlay"), Sense::click_and_drag());
            if units.abs() > 1.0 {
                let time = ui.input(|i| i.time);
                let steps = wheel_a.steps(time, units);
                if steps != 0 {
                    if let Some(pos) = pointer {
                        let (z, p) =
                            zoom_about(*zoom_a, *pan_a, pos - canvas.center(), wheel_factor(steps));
                        *zoom_a = z;
                        *pan_a = p;
                    }
                }
            }
            if resp.dragged() {
                *pan_a += resp.drag_delta();
            }
            if resp.double_clicked() {
                if *zoom_a > 1.01 {
                    *zoom_a = 1.0;
                    *pan_a = Vec2::ZERO;
                } else {
                    *zoom_a = 2.0;
                    *pan_a = Vec2::ZERO;
                }
            }
            if let Some(ta) = tex_a.as_ref() {
                let size = fitted_size(ta.size(), *zoom_a, canvas.size());
                *pan_a = clamp_pan(*pan_a, size, canvas.size());
            }
            match tex_a.as_ref() {
                Some(ta) => draw_fit(ui, canvas, ta, *zoom_a, *pan_a),
                None => loading(ui, canvas),
            }
            if let Some(tb) = tex_b.as_ref() {
                let alpha = cmp.opacity as f32 / 100.0;
                draw_fit_alpha(ui, canvas, tb, *zoom_a, *pan_a, alpha);
            }
            let caption = format!(
                "{} + {} ({}%)",
                file_label(&cmp.a),
                file_label(&cmp.b),
                cmp.opacity
            );
            let font = FontId::proportional(10.5);
            let w = ui
                .painter()
                .layout_no_wrap(caption.clone(), font.clone(), theme::text())
                .size()
                .x;
            let tab = Rect::from_min_size(
                Pos2::new(canvas.left() + 10.0, canvas.top() + 6.0),
                Vec2::new(w + 12.0, 18.0),
            );
            ui.painter()
                .rect_filled(tab, CornerRadius::same(3), Color32::from_black_alpha(170));
            ui.painter().text(
                Pos2::new(tab.left() + 6.0, tab.center().y),
                Align2::LEFT_CENTER,
                caption,
                font,
                theme::on_scrim(),
            );
        }
    }
}

fn loading(ui: &egui::Ui, pane: Rect) {
    ui.painter().text(
        pane.center(),
        Align2::CENTER_CENTER,
        "Loading…",
        FontId::proportional(12.0),
        theme::text_dim(),
    );
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
    let size = fitted_size(tex.size(), zoom, canvas.size());
    if size == Vec2::ZERO {
        return Rect::from_center_size(canvas.center(), Vec2::ZERO);
    }
    Rect::from_center_size(canvas.center() + pan, size)
}

/// A picture inside its own pane, and never outside it.
fn draw_fit(ui: &egui::Ui, canvas: Rect, tex: &egui::TextureHandle, zoom: f32, pan: Vec2) {
    let painter = ui.painter().with_clip_rect(canvas);
    painter.image(
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
    let painter = ui.painter().with_clip_rect(canvas);
    painter.image(
        tex.id(),
        fit_rect(canvas, tex, zoom, pan),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8),
    );
}

/// One picture framed in the whole canvas, shown only inside `clip` — the two
/// halves of the Split comparison.
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
