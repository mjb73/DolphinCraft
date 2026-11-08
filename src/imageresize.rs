//! Added feature, asked for this round: *Resize Image…* in the right-click menu.
//!
//! The person's words: "add a new feature in the context menu to resize an
//! image keeping the aspect ratio but specifying MP".
//!
//! So the one number the window asks for is a size in **megapixels**, not a
//! width or a height. Everything else follows from it: the long edge and the
//! short edge are both scaled by the same factor, so the picture keeps its shape
//! exactly, and only the number of pixels changes.
//!
//! Nothing here is copied from Dolphin: the original has no resize action at
//! all. Its right-click menu on a picture offers *Open With*, *Open with
//! <program>*, the KIPI/Service-menu plugins, *Properties* and the file
//! operations (`strings /usr/bin/dolphin` and
//! `/usr/lib/libdolphinprivate.so.6` hold no "Resize", "Rotate" or "megapixel"
//! interface text — see MECHANICS.md). What *is* taken from the original is the
//! unit: its own size caption prints `W×H:AR a.bc:MP d.dd`, and that `MP` is
//! width × height ÷ 1 000 000 with two decimals — the same reading of a
//! megapixel this module uses.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader};

/// One megapixel, the way the original's size caption counts it.
pub const PIXELS_PER_MP: f64 = 1_000_000.0;

/// The size the window suggests for a picture that has plenty of pixels. Two
/// megapixels is 1920×1080 to within a rounding step — the size a picture is
/// usually wanted at when it is going to a screen, a message or a form.
pub const DEFAULT_MP: f64 = 2.0;

/// The smallest target the field accepts: 0.01 MP is 100×100 for a square
/// picture. Below that a picture stops being a picture.
pub const MIN_MP: f64 = 0.01;

/// The largest target the field accepts. 200 MP is a 16 000 × 12 500 picture,
/// which is already 800 MB as raw pixels; a bigger target than this would fail
/// on memory rather than resize.
pub const MAX_MP: f64 = 200.0;

/// JPEG quality the window writes at. PNG, WebP, BMP and TIFF are written
/// without a quality setting.
pub const DEFAULT_QUALITY: u8 = 90;

/// The extensions this window reads back and writes out.
pub const SUPPORTED: [&str; 7] = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];

/// The filter the scaling uses: the sharpest of the crate's resamplers, and the
/// one every picture tool offers by that name.
pub const FILTER: image::imageops::FilterType = image::imageops::FilterType::Lanczos3;

pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Whether this window can write a picture with that extension back out.
pub fn supported(path: &Path) -> bool {
    SUPPORTED.contains(&extension(path).as_str())
}

/// The pixel count of a picture in megapixels, counted the way the original's
/// caption counts it: width × height ÷ 1 000 000.
pub fn megapixels(w: u32, h: u32) -> f64 {
    (w as f64 * h as f64) / PIXELS_PER_MP
}

/// "2", "2.5", "0.25" — the number as the person would type it, without the
/// trailing zeros a fixed two-decimal format would leave behind.
pub fn format_mp(mp: f64) -> String {
    if !mp.is_finite() || mp <= 0.0 {
        return DEFAULT_MP.to_string();
    }
    let mut text = format!("{mp:.2}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Read the megapixels out of the field. Accepts "2", "2.5", "2,5" (a comma
/// decimal, as some keyboards write it) and "2 MP".
pub fn parse_mp(text: &str) -> Result<f64, String> {
    let cleaned = text
        .trim()
        .to_lowercase()
        .replace("megapixels", "")
        .replace("megapixel", "")
        .replace("mp", "")
        .replace(',', ".")
        .replace(' ', "");
    if cleaned.is_empty() {
        return Err("Type how many megapixels, for example 2 or 2.5.".into());
    }
    let mp: f64 = cleaned
        .parse()
        .map_err(|_| "That is not a number. Type how many megapixels, for example 2 or 2.5.".to_string())?;
    if !mp.is_finite() {
        return Err("That is not a number. Type how many megapixels, for example 2 or 2.5.".into());
    }
    if mp < MIN_MP {
        return Err(format!(
            "The smallest size this window makes is {} MP.",
            format_mp(MIN_MP)
        ));
    }
    if mp > MAX_MP {
        return Err(format!(
            "The largest size this window makes is {} MP.",
            format_mp(MAX_MP)
        ));
    }
    Ok(mp)
}

/// Whether an Exif orientation swaps the two sides.
fn turns(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
    )
}

/// The size a picture has to be so its pixel count lands on `mp` while its
/// shape stays exactly as it was.
///
/// The factor is `sqrt(target ÷ now)`, because the pixel count grows with the
/// square of the side. The width is rounded to the nearest pixel and the height
/// is then taken from the width at the picture's own ratio, so the two sides can
/// never drift apart: a 4:3 picture stays 4:3 to the pixel, and the count lands
/// within a rounding step of the number asked for.
pub fn target_dims(w: u32, h: u32, mp: f64) -> (u32, u32) {
    if w == 0 || h == 0 || !mp.is_finite() || mp <= 0.0 {
        return (w.max(1), h.max(1));
    }
    let now = megapixels(w, h);
    let scale = (mp / now).sqrt();
    let tw = ((w as f64) * scale).round().max(1.0) as u32;
    let th = (((tw as f64) * (h as f64) / (w as f64)).round().max(1.0)) as u32;
    (tw, th)
}

/// The picture's own size, read from the file's header without decoding the
/// pixels. An Exif-turned photograph is reported the way it is seen (upright).
pub fn probe(path: &Path) -> Result<(u32, u32), String> {
    let reader = ImageReader::open(path).map_err(|e| format!("The picture could not be opened: {e}"))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| format!("The picture could not be read: {e}"))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("The picture could not be read: {e}"))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Err("The file says its picture is empty.".into());
    }
    Ok(if turns(orientation) { (h, w) } else { (w, h) })
}

/// `photo-2MP.jpg` beside the original; the original itself when `replace` is
/// set. A name already taken steps aside to `photo-2MP (1).jpg` rather than
/// overwrite anything.
pub fn output_path(src: &Path, mp: f64, replace: bool) -> PathBuf {
    if replace {
        return src.to_path_buf();
    }
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "picture".to_string());
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let want = format!("{stem}-{}MP", format_mp(mp));
    let name = if ext.is_empty() {
        want
    } else {
        format!("{want}.{ext}")
    };
    step_aside(&src.with_file_name(name))
}

/// The first free name of the same shape: `x.jpg`, `x (1).jpg`, `x (2).jpg`…
fn step_aside(want: &Path) -> PathBuf {
    if !want.exists() {
        return want.to_path_buf();
    }
    let stem = want
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "picture".to_string());
    let ext = want
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    for n in 1..1000 {
        let name = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let candidate = want.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    want.to_path_buf()
}

/// A half-written picture never takes the place of a good one: the image is
/// encoded into a `part` file beside the target first and only renamed over it
/// once the encoder has finished.
fn part_path(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "picture".to_string());
    out.with_file_name(format!(".{name}.part-{}", std::process::id()))
}

/// Write the picture in the format its extension asks for.
fn write_picture(img: &DynamicImage, out: &Path, quality: u8) -> Result<(), String> {
    let ext = extension(out);
    let part = part_path(out);
    let result = (|| -> Result<(), String> {
        match ext.as_str() {
            "jpg" | "jpeg" => {
                // JPEG has no alpha channel: the transparent parts of a WebP or
                // PNG are flattened onto white, not onto black.
                let rgb = img.to_rgb8();
                let file = File::create(&part).map_err(|e| format!("The new picture could not be written: {e}"))?;
                let mut encoder =
                    JpegEncoder::new_with_quality(BufWriter::new(file), quality.clamp(1, 100));
                encoder
                    .encode_image(&rgb)
                    .map_err(|e| format!("The new picture could not be written: {e}"))?;
            }
            "webp" => {
                let rgba = img.to_rgba8();
                let file = File::create(&part).map_err(|e| format!("The new picture could not be written: {e}"))?;
                WebPEncoder::new_lossless(BufWriter::new(file))
                    .encode(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| format!("The new picture could not be written: {e}"))?;
            }
            "png" => img
                .save_with_format(&part, ImageFormat::Png)
                .map_err(|e| format!("The new picture could not be written: {e}"))?,
            "bmp" => img
                .save_with_format(&part, ImageFormat::Bmp)
                .map_err(|e| format!("The new picture could not be written: {e}"))?,
            "tif" | "tiff" => img
                .save_with_format(&part, ImageFormat::Tiff)
                .map_err(|e| format!("The new picture could not be written: {e}"))?,
            other => {
                return Err(format!(
                    "{} is not a format this window writes.",
                    other.to_uppercase()
                ))
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            fs::rename(&part, out).map_err(|e| {
                let _ = fs::remove_file(&part);
                format!("The new picture was made but could not be put in place: {e}")
            })?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Decode `src`, scale it to `to` (a size that already has the picture's shape)
/// and write the result to `out`.
pub fn resize_file(src: &Path, to: (u32, u32), out: &Path, quality: u8) -> Result<(), String> {
    if to.0 == 0 || to.1 == 0 {
        return Err("The new size came out as nothing.".into());
    }
    if (to.0 as f64) * (to.1 as f64) > MAX_MP * PIXELS_PER_MP {
        return Err(format!(
            "The new picture would be larger than {} MP.",
            format_mp(MAX_MP)
        ));
    }
    let reader = ImageReader::open(src).map_err(|e| format!("The picture could not be opened: {e}"))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| format!("The picture could not be read: {e}"))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("The picture could not be read: {e}"))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("The picture could not be decoded: {e}"))?;
    // A phone photograph is stored sideways with a note saying how to turn it;
    // turning it here means the new file looks like the one on screen.
    img.apply_orientation(orientation);
    let scaled = img.resize_exact(to.0, to.1, FILTER);
    write_picture(&scaled, out, quality)
}

/// One picture in the window: where it is, how big it is, and — when it cannot
/// be resized — the sentence saying why.
pub struct Item {
    pub src: PathBuf,
    pub from: Option<(u32, u32)>,
    pub note: Option<String>,
}

impl Item {
    fn read(src: PathBuf) -> Self {
        if !src.is_file() {
            return Self {
                src,
                from: None,
                note: Some("This is not a file.".into()),
            };
        }
        if !supported(&src) {
            let ext = extension(&src).to_uppercase();
            let note = if ext.is_empty() {
                "This file has no picture extension, so its format is unknown.".to_string()
            } else if matches!(
                ext.as_str(),
                "GIF" | "SVG" | "ICO" | "HEIC" | "AVIF" | "XPM" | "PPM" | "JFIF"
            ) {
                format!("{ext} pictures are not resized here — PNG, JPEG, WebP, BMP and TIFF are.")
            } else {
                format!("This is not a picture this window can read ({ext} file).")
            };
            return Self {
                src,
                from: None,
                note: Some(note),
            };
        }
        match probe(&src) {
            Ok(dims) => Self {
                src,
                from: Some(dims),
                note: None,
            },
            Err(e) => Self {
                src,
                from: None,
                note: Some(e),
            },
        }
    }

    pub fn name(&self) -> String {
        self.src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.src.display().to_string())
    }
}

/// What one run will do to one picture, decided when the button is pressed.
pub struct Plan {
    pub src: PathBuf,
    pub from: (u32, u32),
    pub to: (u32, u32),
    pub out: PathBuf,
}

/// A finished run: the pictures written, the ones that failed, and why.
pub struct Outcome {
    pub written: Vec<PathBuf>,
    pub failures: Vec<(PathBuf, String)>,
    pub total: usize,
}

impl Outcome {
    /// The sentence the toast shows afterwards.
    pub fn message(&self, mp: f64) -> String {
        let n = self.written.len();
        if n == 0 {
            let why = self
                .failures
                .first()
                .map(|(p, e)| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    format!("{name}: {e}")
                })
                .unwrap_or_else(|| "nothing could be written".into());
            return format!("No picture was resized — {why}");
        }
        let size = format_mp(mp);
        if self.failures.is_empty() {
            if n == 1 {
                format!("Picture resized to {size} MP.")
            } else {
                format!("{n} pictures resized to {size} MP each.")
            }
        } else {
            format!(
                "{} of {} pictures resized to {size} MP; {} could not be.",
                n,
                self.total,
                self.failures.len()
            )
        }
    }
}

/// The work handed to a worker thread: one picture after another, so a batch of
/// forty photographs does not decode forty at once.
pub struct Work {
    pub total: usize,
    pub done: usize,
    pub written: Vec<PathBuf>,
    pub failures: Vec<(PathBuf, String)>,
    pub finished: bool,
    rx: Receiver<(PathBuf, Result<PathBuf, String>)>,
}

/// The *Resize Image* window's state.
pub struct Resize {
    pub items: Vec<Item>,
    /// What the person typed into the megapixel field.
    pub mp_text: String,
    pub quality: u8,
    /// Overwrite the original instead of writing a copy beside it.
    pub replace: bool,
    /// The field takes the keyboard on the first frame.
    pub focus: bool,
    pub error: Option<String>,
    pub work: Option<Work>,
    /// The megapixels the run was started with, for the sentence afterwards.
    pub last_mp: f64,
}

impl Resize {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        let items: Vec<Item> = paths.into_iter().map(Item::read).collect();
        // The field starts at 2 MP, or at the picture's own size when it is
        // smaller than that: opening the window on a small picture should never
        // offer to blow it up.
        let mp_text = match items.iter().find_map(|i| i.from) {
            Some((w, h)) => {
                let now = megapixels(w, h);
                if now < DEFAULT_MP {
                    format_mp(((now * 10.0).floor() / 10.0).max(MIN_MP))
                } else {
                    format_mp(DEFAULT_MP)
                }
            }
            None => format_mp(DEFAULT_MP),
        };
        Self {
            items,
            mp_text,
            quality: DEFAULT_QUALITY,
            replace: false,
            focus: true,
            error: None,
            work: None,
            last_mp: DEFAULT_MP,
        }
    }

    pub fn mp(&self) -> Result<f64, String> {
        parse_mp(&self.mp_text)
    }

    /// The pictures that can be resized.
    pub fn ready(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.from.is_some())
    }

    /// The first picture's own size, for the read-out at the top.
    pub fn first_size(&self) -> Option<(u32, u32)> {
        self.ready().next().and_then(|i| i.from)
    }

    /// What the target means for the first picture, for the live read-out.
    pub fn preview(&self) -> Option<((u32, u32), (u32, u32))> {
        let mp = self.mp().ok()?;
        let (w, h) = self.first_size()?;
        Some(((w, h), target_dims(w, h, mp)))
    }

    /// Whether any picture in the window is a JPEG, which decides whether the
    /// quality slider is worth showing.
    pub fn any_jpeg(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(extension(&i.src).as_str(), "jpg" | "jpeg"))
    }

    pub fn running(&self) -> bool {
        self.work.as_ref().map(|w| !w.finished).unwrap_or(false)
    }

    /// Every picture the button will write, with its target size and file name.
    pub fn plan(&self) -> Result<Vec<Plan>, String> {
        let mp = self.mp()?;
        let mut out = Vec::new();
        for item in self.ready() {
            let Some((w, h)) = item.from else { continue };
            let to = target_dims(w, h, mp);
            out.push(Plan {
                src: item.src.clone(),
                from: (w, h),
                to,
                out: output_path(&item.src, mp, self.replace),
            });
        }
        if out.is_empty() {
            return Err("No picture in this selection can be resized here.".into());
        }
        Ok(out)
    }

    pub fn begin(&mut self) -> bool {
        if self.running() {
            return false;
        }
        self.error = None;
        let plans = match self.plan() {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        let mp = self.mp().unwrap_or(self.last_mp);
        self.last_mp = mp;
        let total = plans.len();
        let quality = self.quality;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for plan in plans {
                let result = resize_file(&plan.src, plan.to, &plan.out, quality);
                if tx.send((plan.src.clone(), result.map(|_| plan.out.clone()))).is_err() {
                    return;
                }
            }
        });
        self.work = Some(Work {
            total,
            done: 0,
            written: Vec::new(),
            failures: Vec::new(),
            finished: false,
            rx,
        });
        true
    }

    /// Take what the worker thread has sent since the last frame. `Some` once
    /// every picture has been through it.
    pub fn poll(&mut self) -> Option<Outcome> {
        let work = self.work.as_mut()?;
        loop {
            match work.rx.try_recv() {
                Ok((src, result)) => {
                    work.done += 1;
                    match result {
                        Ok(path) => work.written.push(path),
                        Err(e) => work.failures.push((src, e)),
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    work.done = work.total;
                    break;
                }
            }
        }
        if work.done >= work.total && !work.finished {
            work.finished = true;
        }
        if work.finished {
            Some(Outcome {
                written: work.written.clone(),
                failures: work.failures.clone(),
                total: work.total,
            })
        } else {
            None
        }
    }
}
