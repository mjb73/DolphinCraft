//! Background work: thumbnails, recursive search, duplicate detection,
//! folder sizes and checksums.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::fsutil::{self, Entry};

// ---------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------

struct ThumbJob {
    path: PathBuf,
    size: u32,
}

struct ThumbResult {
    path: PathBuf,
    size: u32,
    image: Option<(usize, usize, Vec<u8>)>,
}

/// Where finished previews are kept between runs, mirroring the original's
/// `~/.cache/thumbnails` so a folder of photos is instant the second time it is
/// opened and a larger zoom step only decodes what it has not seen before.
fn thumb_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let dir = base.join("file-craft").join("thumbs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// A name for one (file, version, size) preview. The modification time is part
/// of it, so an edited picture never shows its old preview.
fn thumb_cache_key(path: &Path, size: u32) -> Option<String> {
    use sha2::{Digest, Sha256};
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let mut h = Sha256::new();
    h.update(path.as_os_str().as_encoded_bytes());
    h.update(mtime.to_le_bytes());
    h.update(size.to_le_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(48);
    for b in digest.iter().take(24) {
        out.push_str(&format!("{:02x}", b));
    }
    Some(out)
}

/// Drop previews nobody has looked at for two months, so the cache cannot grow
/// without bound on a disk that is nearly full.
fn prune_thumb_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        let stale = md
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|age| age.as_secs() > 60 * 24 * 60 * 60)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Default)]
struct ThumbEntry {
    texture: Option<egui::TextureHandle>,
    /// Pixel size the current texture was rendered at.
    tex_size: u32,
    /// Size of a job that is queued or still being worked on.
    pending: Option<u32>,
    /// Largest size ever asked for; a small source image stops at its own size
    /// and must not be re-queued every frame.
    asked: u32,
    failed: bool,
    /// Byte size of the file, read once. `[PreviewSettings] MaximumSize` decides
    /// from it whether a preview may be made at all.
    bytes: Option<u64>,
}

pub struct ThumbCache {
    entries: HashMap<PathBuf, ThumbEntry>,
    /// Pixel size of each picture, read once from the file header. The icons
    /// view needs it *before* it draws anything, so that a tall picture can be
    /// given a tall cell instead of a square one; decoding the whole picture to
    /// learn its shape would stall the first frame of a photo folder.
    sizes: HashMap<PathBuf, Option<(u32, u32)>>,
    queue: Arc<Mutex<VecDeque<ThumbJob>>>,
    rx: Receiver<ThumbResult>,
    requested: usize,
    /// `[PreviewSettings] MaximumSize` in MiB; 0 means "files of any size".
    max_mib: u32,
}

/// Walk a PNG's chunk list until the image header turns up, and read the width
/// and height out of it. Stops at the pixel data: an IHDR that is not before
/// IDAT is not a PNG a decoder would accept either.
fn png_size(f: &mut std::fs::File) -> Option<(u32, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(8)).ok()?;
    let mut hdr = [0u8; 8];
    for _ in 0..64 {
        f.read_exact(&mut hdr).ok()?;
        let len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let kind = &hdr[4..8];
        if kind == b"IHDR" {
            let mut data = [0u8; 8];
            f.read_exact(&mut data).ok()?;
            let w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            return if w > 0 && h > 0 { Some((w, h)) } else { None };
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        // Skip the chunk's own bytes and its four-byte checksum.
        f.seek(SeekFrom::Current(len as i64 + 4)).ok()?;
    }
    None
}

/// Pixel size of a picture read from its own header, without decoding pixels.
/// Covers the formats the previews handle: JPEG, PNG, GIF, BMP and WebP.
pub fn probe_image_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 32];
    let n = f.read(&mut head).ok()?;
    if n < 10 {
        return None;
    }
    // PNG: 8-byte signature, then a chunk list. IHDR is meant to come first,
    // but a picture that carries a prompt can have its text chunk written
    // before it (or any other ancillary chunk can), so the header is hunted
    // for rather than assumed to sit at a fixed offset — reading the width out
    // of whatever happened to be there gave the size line numbers in the
    // billions.
    if head[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        if let Some(wh) = png_size(&mut f) {
            return Some(wh);
        }
    }
    // GIF: "GIF87a"/"GIF89a", then little-endian width and height.
    if &head[..3] == b"GIF" {
        let w = u16::from_le_bytes([head[6], head[7]]) as u32;
        let h = u16::from_le_bytes([head[8], head[9]]) as u32;
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    // BMP: little-endian width and height after the 14-byte file header.
    if head[..2] == *b"BM" && n >= 26 {
        let w = i32::from_le_bytes([head[18], head[19], head[20], head[21]]);
        let h = i32::from_le_bytes([head[22], head[23], head[24], head[25]]).abs();
        if w > 0 && h > 0 {
            return Some((w as u32, h as u32));
        }
    }
    // WebP: RIFF container; the canvas size sits in the first chunk header.
    if head[..4] == *b"RIFF" && &head[8..12] == b"WEBP" {
        match &head[12..16] {
            b"VP8X" => {
                let w = 1 + u32::from_le_bytes([head[24], head[25], head[26], 0]);
                let h = 1 + u32::from_le_bytes([head[27], head[28], head[29], 0]);
                return Some((w, h));
            }
            b"VP8 " => {
                // Lossy: 3-byte frame tag, 3-byte start code, then 14-bit size.
                if head[23..26] == [0x9d, 0x01, 0x2a] {
                    let w = u16::from_le_bytes([head[26], head[27]]) & 0x3fff;
                    let h = u16::from_le_bytes([head[28], head[29]]) & 0x3fff;
                    if w > 0 && h > 0 {
                        return Some((w as u32, h as u32));
                    }
                }
            }
            b"VP8L" => {
                let b = &head[21..25];
                let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let w = (bits & 0x3fff) + 1;
                let h = ((bits >> 14) & 0x3fff) + 1;
                return Some((w, h));
            }
            _ => {}
        }
    }
    // JPEG: walk the markers to the first frame header (SOF0..SOF15, skipping
    // the Huffman and arithmetic tables that share the same range).
    if head[..2] == [0xff, 0xd8] {
        let mut f = std::fs::File::open(path).ok()?;
        let mut buf = Vec::new();
        f.take(256 * 1024).read_to_end(&mut buf).ok()?;
        let mut i = 2usize;
        while i + 9 < buf.len() {
            if buf[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = buf[i + 1];
            if marker == 0xff {
                i += 1;
                continue;
            }
            if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
            if (0xc0..=0xcf).contains(&marker)
                && marker != 0xc4
                && marker != 0xc8
                && marker != 0xcc
                && i + 9 <= buf.len()
            {
                let h = u16::from_be_bytes([buf[i + 5], buf[i + 6]]) as u32;
                let w = u16::from_be_bytes([buf[i + 7], buf[i + 8]]) as u32;
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
                return None;
            }
            if marker == 0xda {
                break;
            }
            i += 2 + len;
        }
    }
    None
}

impl ThumbCache {
    pub fn new() -> Self {
        let queue: Arc<Mutex<VecDeque<ThumbJob>>> = Arc::new(Mutex::new(VecDeque::new()));
        let (tx, rx) = channel::<ThumbResult>();
        // One decoder per core (capped at eight): a folder of phone photos can
        // hold hundreds of multi-megabyte pictures, and the window should fill
        // its previews while the person is still looking at the first row.
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 8);
        let cache_dir = thumb_cache_dir();
        if let Some(dir) = cache_dir.clone() {
            std::thread::spawn(move || prune_thumb_cache(&dir));
        }
        for _ in 0..workers {
            let q = queue.clone();
            let tx = tx.clone();
            let cache_dir = cache_dir.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let mut lock = q.lock().unwrap();
                    lock.pop_front()
                };
                let Some(job) = job else {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    continue;
                };
                let key = thumb_cache_key(&job.path, job.size);
                let cache_file = match (&cache_dir, &key) {
                    (Some(dir), Some(key)) => Some(dir.join(format!("{key}.png"))),
                    _ => None,
                };
                // A preview rendered before is taken from the on-disk cache; only
                // new sizes pay for decoding the original picture.
                let mut thumb = cache_file
                    .as_ref()
                    .and_then(|p| image::open(p).ok())
                    .and_then(|img| {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        Some((w, h, rgba.into_raw()))
                    });
                if thumb.is_none() {
                    // A picture decodes here; a video has no decoder in the
                    // `image` crate, so its first frame is taken by ffmpeg — the
                    // program the machine already has for video.
                    let frame = image::open(&job.path).ok().or_else(|| {
                        if crate::fsutil::is_video_path(&job.path) {
                            crate::video::first_frame(&job.path, job.size)
                        } else {
                            None
                        }
                    });
                    if let Some(img) = frame {
                        let rgba = img.thumbnail(job.size, job.size).to_rgba8();
                        let (w, h) = rgba.dimensions();
                        if let Some(p) = cache_file.as_ref() {
                            let _ = image::save_buffer(
                                p,
                                rgba.as_raw(),
                                w,
                                h,
                                image::ColorType::Rgba8,
                            );
                        }
                        thumb = Some((w, h, rgba.into_raw()));
                    }
                }
                let image = thumb.map(|(w, h, raw)| (w as usize, h as usize, raw));
                if tx
                    .send(ThumbResult {
                        path: job.path,
                        size: job.size,
                        image,
                    })
                    .is_err()
                {
                    return;
                }
            });
        }
        Self {
            entries: HashMap::new(),
            sizes: HashMap::new(),
            queue,
            rx,
            requested: 0,
            max_mib: 0,
        }
    }

    /// Apply `[PreviewSettings] MaximumSize` in MiB from the settings. Lowering
    /// the cap takes effect on the next frame: a file that is now too big is
    /// simply never queued, and the view draws its type icon instead.
    pub fn set_max_mib(&mut self, mib: u32) {
        self.max_mib = mib;
    }

    /// Width divided by height of a picture, or `1.0` when the file is not a
    /// picture or its header cannot be read. Read once per path.
    pub fn aspect(&mut self, path: &Path) -> f32 {
        let size = *self
            .sizes
            .entry(path.to_path_buf())
            .or_insert_with(|| probe_image_size(path));
        match size {
            Some((w, h)) if w > 0 && h > 0 => w as f32 / h as f32,
            _ => 1.0,
        }
    }

    /// Ask for a thumbnail at least `size` pixels along its longest edge and
    /// return the best texture available right now.
    ///
    /// The original rescales its cached preview whenever the icon size changes
    /// (`KStandardItemListWidget::drawPixmap` @0x135bb0 remembers the cached
    /// width and height in the widget and calls `KPixmapModifier::scale` when
    /// they differ from the current icon rect), so zooming in always gives a
    /// bigger thumbnail. A texture rendered for a smaller zoom level is
    /// therefore too small and is re-rendered; a bigger one is reused and
    /// scaled down by the caller. While the new job runs the old texture is
    /// returned so the cell never goes blank.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        size: u32,
    ) -> Option<egui::TextureHandle> {
        let size = size.clamp(16, 2048);
        self.drain(ctx);
        let entry = self.entries.entry(path.to_path_buf()).or_default();
        if entry.failed {
            return None;
        }
        // The original's "Show previews for files below N MiB": a file over
        // `[PreviewSettings] MaximumSize` never reaches the thumbnail job, so the
        // view draws its kind icon instead. 0 = no cap (the box's "files of any
        // size"), and the file's length is read once per path.
        if self.max_mib > 0 {
            let bytes = *entry
                .bytes
                .get_or_insert_with(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
            if !preview_allowed(bytes, self.max_mib) {
                return entry.texture.clone();
            }
        }
        let wants_bigger = entry.texture.is_none() || entry.tex_size < size;
        if wants_bigger && entry.pending.is_none() && entry.asked < size && self.requested < 4000 {
            let mut q = self.queue.lock().unwrap();
            if q.len() < 600 {
                q.push_back(ThumbJob {
                    path: path.to_path_buf(),
                    size,
                });
                entry.pending = Some(size);
                entry.asked = size;
                self.requested += 1;
            }
        }
        entry.texture.clone()
    }

    /// Promote finished worker results into textures.
    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(res) = self.rx.try_recv() {
            let entry = self.entries.entry(res.path.clone()).or_default();
            if entry.pending == Some(res.size) {
                entry.pending = None;
            }
            match res.image {
                Some((w, h, rgba)) => {
                    let pixels = rgba
                        .chunks_exact(4)
                        .map(|c| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
                        .collect();
                    let color = egui::ColorImage::new([w, h], pixels);
                    let handle = ctx.load_texture(
                        format!("{}#{}", res.path.to_string_lossy(), res.size),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    entry.texture = Some(handle);
                    entry.tex_size = res.size;
                    entry.failed = false;
                }
                None => {
                    if entry.texture.is_none() {
                        entry.failed = true;
                    }
                }
            }
        }
    }

    /// Drain results even when nothing is asked for this frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.drain(ctx);
        // Keep the frame clock running while work is queued or a bigger
        // thumbnail is on its way, so the zoom change shows up by itself.
        let busy = self.entries.values().any(|e| e.pending.is_some())
            || !self.queue.lock().map(|q| q.is_empty()).unwrap_or(true);
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.remove(path);
        self.sizes.remove(path);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.sizes.clear();
        self.requested = 0;
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct SearchFilters {
    pub min_size: Option<u64>,
    pub max_age_hours: Option<u64>,
    pub folders_only: bool,
    /// Added feature: keep pictures only. The picture wall's "Sub-folders"
    /// switch starts a walk with this on, so the wall can show the pictures of
    /// a whole tree — a folder of year or month folders — with every other file
    /// and folder left out of the result.
    pub pictures_only: bool,
}

pub enum SearchEvent {
    Batch(Vec<Entry>),
    Done,
    Failed(String),
}

pub struct SearchSession {
    pub query: String,
    pub root: PathBuf,
    pub filters: SearchFilters,
    pub rx: Receiver<SearchEvent>,
    pub results: Vec<Entry>,
    pub done: bool,
    pub error: Option<String>,
    /// Added feature: this walk is not the find bar's search but the picture
    /// wall looking below the folder. The difference matters in three places:
    /// the wall's own wording, the status bar's, and the fact that the find bar
    /// stays shut, because nothing was typed.
    pub pictures_below: bool,
}

impl SearchSession {
    pub fn start(root: PathBuf, query: String, filters: SearchFilters) -> Self {
        let (tx, rx) = channel();
        let r = root.clone();
        let q = query.clone();
        let f = filters.clone();
        std::thread::spawn(move || {
            let mut batch = Vec::new();
            let mut count = 0;
            let needle = q.to_lowercase();
            let walker = walkdir::WalkDir::new(&r)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    e.depth() == 0 || !name.starts_with('.')
                });
            for item in walker.flatten() {
                if item.depth() == 0 {
                    continue;
                }
                if count >= 4000 {
                    break;
                }
                let name = item.file_name().to_string_lossy().to_string();
                if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                    continue;
                }
                let entry = Entry::from_path(item.path());
                if f.folders_only && !entry.is_dir {
                    continue;
                }
                // A wall of pictures is what asked for this walk, so folders
                // and every non-picture file stay out of the result.
                if f.pictures_only && (entry.is_dir || !entry.is_image()) {
                    continue;
                }
                if let Some(min) = f.min_size {
                    if entry.size < min {
                        continue;
                    }
                }
                if let Some(hours) = f.max_age_hours {
                    if !fsutil::modified_within(&entry, hours) {
                        continue;
                    }
                }
                count += 1;
                batch.push(entry);
                if batch.len() >= 32 {
                    if tx.send(SearchEvent::Batch(std::mem::take(&mut batch))).is_err() {
                        return;
                    }
                }
            }
            if !batch.is_empty() {
                let _ = tx.send(SearchEvent::Batch(batch));
            }
            let _ = tx.send(SearchEvent::Done);
        });
        Self {
            query,
            root,
            filters,
            rx,
            results: Vec::new(),
            done: false,
            error: None,
            pictures_below: false,
        }
    }

    /// Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                SearchEvent::Batch(items) => {
                    self.results.extend(items);
                    changed = true;
                }
                SearchEvent::Done => {
                    self.done = true;
                    changed = true;
                }
                SearchEvent::Failed(e) => {
                    self.error = Some(e);
                    self.done = true;
                    changed = true;
                }
            }
        }
        changed
    }
}

// ---------------------------------------------------------------------------
// Duplicate finder
// ---------------------------------------------------------------------------

pub struct DupGroup {
    pub size: u64,
    pub hash: String,
    pub files: Vec<PathBuf>,
}

pub enum DupEvent {
    Group(DupGroup),
    Progress(usize),
    Done,
}

pub struct DupScan {
    pub rx: Receiver<DupEvent>,
    pub groups: Vec<DupGroup>,
    pub scanning: bool,
    pub scanned: usize,
    pub root: PathBuf,
}

impl DupScan {
    pub fn start(root: PathBuf) -> Self {
        let (tx, rx) = channel();
        let r = root.clone();
        std::thread::spawn(move || {
            let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
            let mut seen = 0usize;
            for item in walkdir::WalkDir::new(&r)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    e.depth() == 0 || !name.starts_with('.')
                })
                .flatten()
            {
                if !item.file_type().is_file() {
                    continue;
                }
                let Ok(md) = item.metadata() else { continue };
                let size = md.len();
                seen += 1;
                if seen % 40 == 0 {
                    let _ = tx.send(DupEvent::Progress(seen));
                }
                by_size.entry(size).or_default().push(item.path().to_path_buf());
            }
            let mut wasted = 0u64;
            for (size, paths) in by_size.into_iter().filter(|(_, p)| p.len() > 1) {
                let mut by_hash: HashMap<String, Vec<PathBuf>> = HashMap::new();
                for path in paths {
                    if let Ok(hash) = sha256_file(&path) {
                        by_hash.entry(hash).or_default().push(path);
                    }
                }
                for (hash, files) in by_hash.into_iter().filter(|(_, f)| f.len() > 1) {
                    wasted += size * (files.len() as u64 - 1);
                    if tx
                        .send(DupEvent::Group(DupGroup { size, hash, files }))
                        .is_err()
                    {
                        return;
                    }
                }
            }
            let _ = wasted;
            let _ = tx.send(DupEvent::Done);
        });
        Self {
            rx,
            groups: Vec::new(),
            scanning: true,
            scanned: 0,
            root,
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                DupEvent::Group(g) => {
                    self.groups.push(g);
                    changed = true;
                }
                DupEvent::Progress(n) => {
                    self.scanned = n;
                }
                DupEvent::Done => {
                    self.scanning = false;
                    changed = true;
                }
            }
        }
        if self.scanning {
            changed = true;
        }
        changed
    }

    pub fn wasted(&self) -> u64 {
        self.groups
            .iter()
            .map(|g| g.size * (g.files.len() as u64 - 1))
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Sizes and hashes
// ---------------------------------------------------------------------------

pub fn dir_size(path: &Path) -> u64 {
    dir_size_limited(path, None)
}

/// Factory value of `ContentDisplaySettings::RecursiveDirectorySizeLimit`
/// (ctor @0x2b2b70 in libdolphinprivate.so.6, `ItemUInt(..., 10)`).
pub const SIZE_LIMIT_DEFAULT: u32 = 10;
/// The View → Display page's spin box for that limit: `setRange(1, 20)` and
/// `setSingleStep(1)` at 0x1e5c14 / 0x1e5c23 in `FUN_001e5750` @0x1e5750.
pub const SIZE_LIMIT_MIN: u32 = 1;
pub const SIZE_LIMIT_MAX: u32 = 20;

/// Size of everything under `path`, walking at most `max_depth` folder levels
/// below it.
///
/// `None` walks the whole tree, which is what the duplicate finder and the
/// self-check want. `Some(n)` is the original's "Show size of contents, up to N
/// folders" mode: `ContentDisplaySettings::RecursiveDirectorySizeLimit`
/// (factory 10, allowed 1..20) is handed to the size job as a recursion limit.
/// `walkdir` counts the folder itself as depth 0, so `max_depth(n)` visits n
/// levels below it — the same count the spin box shows.
pub fn dir_size_limited(path: &Path, max_depth: Option<u32>) -> u64 {
    let mut walker = walkdir::WalkDir::new(path).follow_links(false);
    if let Some(depth) = max_depth {
        // `depth` counts subfolder levels below `path`, so one level means the
        // files directly inside `path` plus those one folder down. walkdir counts
        // the folder itself as depth 0, hence the extra step.
        walker = walker.max_depth(depth as usize + 1);
    }
    let mut total = 0u64;
    for item in walker.into_iter().flatten() {
        if item.file_type().is_file() {
            if let Ok(md) = item.metadata() {
                total += md.len();
            }
        }
    }
    total
}

/// Whether a file of `len` bytes may be turned into a thumbnail when the person
/// allows previews for files below `max_mib` MiB.
///
/// This is the rule behind the original's "Show previews for files below N MiB"
/// spin box (`FUN_00217c30` @0x217c30, stored as `[PreviewSettings] MaximumSize`):
/// 0 is the special value the box shows as "files of any size" and means no cap
/// at all; anything else hides the preview from files bigger than N MiB.
pub fn preview_allowed(len: u64, max_mib: u32) -> bool {
    max_mib == 0 || len <= max_mib as u64 * 1024 * 1024
}

pub fn count_items(path: &Path) -> (usize, usize) {
    let mut dirs = 0;
    let mut files = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for item in rd.flatten() {
            if item.path().is_dir() {
                dirs += 1;
            } else {
                files += 1;
            }
        }
    }
    (dirs, files)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// First few KiB of a text file, for Quick Look.
pub fn read_text_preview(path: &Path, max_bytes: usize) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max_bytes];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0u8) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}
