//! Filesystem helpers: reading folders, formatting sizes and dates, the
//! freedesktop trash, mount points and permissions.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, TimeZone};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Folder,
    File,
    Image,
    Audio,
    Video,
    Archive,
    Text,
    Code,
    Pdf,
    Exec,
    Config,
    Link,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub broken: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub mode: u32,
    pub kind: Kind,
    pub hidden: bool,
    /// Immediate children of a folder, counted while the folder is listed.
    pub children: Option<u32>,
    /// Set for folders the person gave a colour of their own.
    pub tint: Option<[u8; 3]>,
    /// Set for items listed from the trash: where they came from.
    pub original: Option<PathBuf>,
}

impl Entry {
    pub fn from_path(path: &Path) -> Entry {
        let md = fs::symlink_metadata(path).ok();
        let is_symlink = md.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false);
        let real_md = if is_symlink { fs::metadata(path).ok() } else { md.clone() };
        let is_dir = real_md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let mode = real_md.as_ref().map(|m| m.mode()).unwrap_or(0);
        let size = real_md.as_ref().map(|m| m.size()).unwrap_or(0);
        let broken = is_symlink && real_md.is_none();
        let kind = kind_for(path, is_dir, mode);
        Entry {
            hidden: name.starts_with('.'),
            path: path.to_path_buf(),
            name,
            is_dir,
            is_symlink,
            broken,
            size,
            modified: real_md.as_ref().and_then(|m| m.modified().ok()),
            accessed: real_md.as_ref().and_then(|m| m.accessed().ok()),
            created: real_md.as_ref().and_then(|m| m.created().ok()),
            mode,
            kind,
            children: None,
            tint: None,
            original: None,
        }
    }

    pub fn type_label(&self) -> String {
        if self.is_dir {
            return "Folder".to_string();
        }
        match self.kind {
            Kind::Image => {
                let ext = self.extension();
                if ext.is_empty() {
                    "Image".to_string()
                } else {
                    format!("{} image", ext.to_uppercase())
                }
            }
            Kind::Audio => "Audio file".to_string(),
            Kind::Video => "Video file".to_string(),
            Kind::Archive => "Archive".to_string(),
            Kind::Text => "Plain text document".to_string(),
            Kind::Code => "Source file".to_string(),
            Kind::Pdf => "PDF document".to_string(),
            Kind::Exec => "Executable".to_string(),
            Kind::Config => "Configuration file".to_string(),
            Kind::Link => {
                if self.is_dir {
                    "Link to folder".to_string()
                } else {
                    "Link to file".to_string()
                }
            }
            _ => match self.extension() {
                e if !e.is_empty() => format!("{} file", e.to_uppercase()),
                _ => "File".to_string(),
            },
        }
    }

    pub fn extension(&self) -> String {
        self.path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }

    pub fn is_image(&self) -> bool {
        matches!(self.kind, Kind::Image)
    }

    pub fn is_text(&self) -> bool {
        matches!(self.kind, Kind::Text | Kind::Code | Kind::Config)
    }
}

fn kind_for(path: &Path, is_dir: bool, mode: u32) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let image = [
        "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "svg", "ico", "heic", "avif",
        "xpm", "ppm",
    ];
    let audio = ["mp3", "flac", "ogg", "oga", "wav", "m4a", "aac", "opus", "wma", "mid"];
    let video = ["mp4", "mkv", "webm", "avi", "mov", "m4v", "wmv", "mpg", "mpeg", "flv"];
    let archive = [
        "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst", "lz4", "tgz", "iso", "cab",
    ];
    let text = ["txt", "md", "markdown", "rst", "log", "org", "nfo", "csv", "tsv"];
    let code = [
        "rs", "py", "js", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "cc", "go", "java", "kt",
        "sh", "bash", "zsh", "fish", "html", "css", "scss", "json", "toml", "yaml", "yml", "xml",
        "sql", "rb", "php", "lua", "swift", "qml", "desktop", "ini", "cfg", "conf", "service",
    ];
    if image.contains(&ext.as_str()) {
        return Kind::Image;
    }
    if audio.contains(&ext.as_str()) {
        return Kind::Audio;
    }
    if video.contains(&ext.as_str()) {
        return Kind::Video;
    }
    if archive.contains(&ext.as_str()) {
        return Kind::Archive;
    }
    if ext == "pdf" {
        return Kind::Pdf;
    }
    if text.contains(&ext.as_str()) {
        return Kind::Text;
    }
    if code.contains(&ext.as_str()) {
        return if ["desktop", "ini", "cfg", "conf", "service", "toml"].contains(&ext.as_str()) {
            Kind::Config
        } else {
            Kind::Code
        };
    }
    if mode & 0o111 != 0 {
        return Kind::Exec;
    }
    Kind::File
}

/// Read a folder. `show_hidden` includes dot files.
pub fn read_folder(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(path).map_err(|e| friendly_error(path, &e))?;
    let mut out = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        out.push(Entry::from_path(&item.path()));
    }
    // Count what is inside each folder so the Size column can say "14 items",
    // the way the original does. Stop at 10000 to stay quick on huge folders.
    for entry in out.iter_mut().filter(|e| e.is_dir) {
        entry.tint = crate::foldercolor::read(&entry.path);
        if let Ok(rd) = fs::read_dir(&entry.path) {
            let mut n = 0u32;
            for _ in rd.flatten() {
                n += 1;
                if n >= 10_000 {
                    break;
                }
            }
            entry.children = Some(n);
        }
    }
    Ok(out)
}

pub fn friendly_error(path: &Path, e: &std::io::Error) -> String {
    let what = match e.kind() {
        std::io::ErrorKind::NotFound => "The location does not exist.",
        std::io::ErrorKind::PermissionDenied => "You do not have permission to view this location.",
        _ => return format!("The location '{}' cannot be opened: {}", path.display(), e),
    };
    format!("{} ({})", what, path.display())
}

pub fn same_day(a: chrono::DateTime<Local>, b: chrono::DateTime<Local>) -> bool {
    a.year() == b.year() && a.ordinal() == b.ordinal()
}

/// Short date, as in the file list. The live capture of the original on this
/// machine shows absolute dates even for files changed minutes earlier
/// ("06/11/2026 18:43"), so no relative wording is used here.
pub fn short_date(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let dt: chrono::DateTime<Local> = t.into();
    dt.format("%d/%m/%Y %H:%M").to_string()
}

/// Date wording used by the Information panel, where the capture shows
/// "14 minute(s) ago" for something just touched, and a plain date otherwise.
pub fn human_date(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let dt: chrono::DateTime<Local> = t.into();
    let now = Local::now();
    let mins = (now - dt).num_minutes();
    if mins < 0 {
        return short_date(Some(t));
    }
    if mins < 1 {
        return "less than a minute ago".to_string();
    }
    if mins < 60 {
        return format!("{} minute(s) ago", mins);
    }
    let hours = mins / 60;
    if same_day(dt, now) {
        return format!("{} hour(s) ago", hours);
    }
    if same_day(dt, now - chrono::Duration::days(1)) && hours < 36 {
        return format!("{} hour(s) ago", hours);
    }
    short_date(Some(t))
}

/// Long date, as in the Information panel: "Friday, 1 May 2026 at 13:00".
pub fn long_date(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let dt: chrono::DateTime<Local> = t.into();
    let day = dt.day();
    format!(
        "{}, {} {} at {}",
        dt.format("%A"),
        day,
        dt.format("%B %Y"),
        dt.format("%H:%M")
    )
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", value, units[unit])
}

/// Share of a volume that is in use, in whole percent.
///
/// `FUN_001e6a40` @0x1e6a40 in `/usr/bin/dolphin` (the function that fills the
/// status bar's free-disk-space button) computes
///
///     pct = (int)(((double)(total - free) * 100.0) / (double)total + 0.5)
///
/// i.e. it rounds half **up**, so 99.5 % used prints as 100 %, while a plain
/// integer division would print 99. A volume of zero size has no percentage
/// and the button is hidden instead.
pub fn used_percent(free: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    let used = total.saturating_sub(free);
    (used as f64 * 100.0 / total as f64 + 0.5) as u64
}

pub fn symbolic_permissions(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        s.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        s.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    s
}

pub fn octal_permissions(mode: u32) -> String {
    format!("{:03o}", mode & 0o777)
}

pub fn user_name(uid: u32) -> String {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return uid.to_string();
        }
        let name = (*pw).pw_name;
        if name.is_null() {
            return uid.to_string();
        }
        std::ffi::CStr::from_ptr(name)
            .to_string_lossy()
            .to_string()
    }
}

pub fn group_name(gid: u32) -> String {
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            return gid.to_string();
        }
        let name = (*gr).gr_name;
        if name.is_null() {
            return gid.to_string();
        }
        std::ffi::CStr::from_ptr(name)
            .to_string_lossy()
            .to_string()
    }
}

/// (free bytes, total bytes) for the filesystem holding `path`.
pub fn free_space(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        let block = st.f_frsize as u64;
        Some((st.f_bavail as u64 * block, st.f_blocks as u64 * block))
    }
}

pub fn mounted_devices() -> Vec<(String, PathBuf)> {
    let mut by_device: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();
    if let Ok(text) = fs::read_to_string("/proc/mounts") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let Some(source) = parts.next() else { continue };
            let Some(mount) = parts.next() else { continue };
            if !source.starts_with("/dev/") || source.starts_with("/dev/loop") {
                continue;
            }
            let mount = mount.replace("\\040", " ");
            if mount.contains("/snap") || mount.contains("/var/lib/") {
                continue;
            }
            let path = PathBuf::from(&mount);
            if !path.is_dir() {
                continue;
            }
            // One entry per device: prefer the shortest mount point, like the
            // Devices section of the original places panel.
            by_device
                .entry(source.to_string())
                .and_modify(|existing| {
                    if mount.len() < existing.as_os_str().len() {
                        *existing = path.clone();
                    }
                })
                .or_insert(path);
        }
    }
    let mut out: Vec<(String, PathBuf)> = by_device
        .into_values()
        .map(|path| {
            let label = match path.to_string_lossy().as_ref() {
                "/" => "File System".to_string(),
                other => other.rsplit('/').next().unwrap_or(other).to_string(),
            };
            (label, path)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Strip `.` and tidy `..` out of a path without touching the filesystem.
///
/// The original shows the folder it is in, not the wording that led there:
/// opening `/usr/share/../lib` puts the location bar on `/usr/lib`. Without
/// this the rebuild's breadcrumb drew every plain component and silently
/// dropped the `..`, so the bar read `usr > share > lib` for a path that is
/// really `/usr/lib`. Virtual locations (`trash:/`) and relative paths are
/// left alone because they are not directories on disk.
pub fn tidy_path(path: &Path) -> PathBuf {
    use std::path::Component;
    if !path.is_absolute() {
        return path.to_path_buf();
    }
    let mut out = PathBuf::from("/");
    for comp in path.components() {
        match comp {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                // `/..` is still `/`; anything else steps back one name.
                out.pop();
            }
            Component::Normal(name) => out.push(name),
            Component::Prefix(_) => {}
        }
    }
    out
}

pub fn xdg_dir(name: &str, fallback: &str) -> PathBuf {
    // ~/.config/user-dirs.dirs is what Dolphin reads for the places panel.
    if let Ok(text) = fs::read_to_string(home_dir().join(".config/user-dirs.dirs")) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(&format!("XDG_{}=", name)) {
                let v = rest.trim().trim_matches('"');
                let v = v.replace("$HOME", &home_dir().to_string_lossy());
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
        }
    }
    home_dir().join(fallback)
}

// ---------------------------------------------------------------------------
// Trash (freedesktop.org trash specification)
// ---------------------------------------------------------------------------

pub fn trash_root() -> PathBuf {
    home_dir().join(".local/share/Trash")
}

pub fn is_trash_path(path: &Path) -> bool {
    path == trash_root() || path.starts_with(trash_root())
}

fn percent_encode_path(path: &Path) -> String {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut out = String::new();
    for b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn unique_trash_name(files: &Path, name: &str) -> String {
    let mut candidate = name.to_string();
    let mut n = 1;
    while files.join(&candidate).exists() || files.join(format!("{}.trashinfo", candidate)).exists()
    {
        // Like the spec: append ".N" before the extension for repeat names.
        let (base, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        candidate = format!("{}.{}{}", base, n, ext);
        n += 1;
    }
    candidate
}

/// Move a file or folder, falling back to copy + remove when the destination is
/// on another filesystem (rename cannot cross devices, and /tmp often differs
/// from the home folder).
fn move_across(src: &Path, dst: &Path) -> std::io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            copy_any(src, dst)?;
            if src.is_dir() {
                fs::remove_dir_all(src)
            } else {
                fs::remove_file(src)
            }
        }
        Err(e) => Err(e),
    }
}

fn copy_any(src: &Path, dst: &Path) -> std::io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    let ft = md.file_type();
    if ft.is_symlink() {
        let link = fs::read_link(src)?;
        std::os::unix::fs::symlink(link, dst)?;
    } else if ft.is_dir() {
        fs::create_dir_all(dst)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_any(&item.path(), &dst.join(item.file_name()))?;
        }
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn move_to_trash(paths: &[PathBuf]) -> Result<usize, String> {
    let root = trash_root();
    let files = root.join("files");
    let info = root.join("info");
    fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let mut moved = 0;
    let mut errors = Vec::new();
    for path in paths {
        let Some(name) = path.file_name().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let target_name = unique_trash_name(&files, &name);
        let target = files.join(&target_name);
        match move_across(path, &target) {
            Ok(()) => {
                let mut f = match fs::File::create(info.join(format!("{}.trashinfo", target_name))) {
                    Ok(f) => f,
                    Err(e) => {
                        errors.push(format!("{}: {}", name, e));
                        continue;
                    }
                };
                let _ = writeln!(
                    f,
                    "[Trash Info]\nPath={}\nDeletionDate={}",
                    percent_encode_path(path),
                    Local::now().format("%Y-%m-%dT%H:%M:%S")
                );
                moved += 1;
            }
            Err(e) => errors.push(format!("{}: {}", name, e)),
        }
    }
    if errors.is_empty() {
        Ok(moved)
    } else {
        Err(errors.join(", "))
    }
}

/// Move paths to the Trash and hand back both what left and the records the
/// move created. The record paths are what an Undo needs: they name exactly the
/// items this action put in the Trash, so a later move cannot be undone by
/// mistake.
pub fn trash_with_records(paths: &[PathBuf]) -> (usize, Vec<PathBuf>, Option<String>) {
    let before: Vec<PathBuf> = list_trash().into_iter().map(|e| e.path).collect();
    let problem = move_to_trash(paths).err();
    let fresh: Vec<PathBuf> = list_trash()
        .into_iter()
        .map(|e| e.path)
        .filter(|p| !before.contains(p))
        .collect();
    (fresh.len(), fresh, problem)
}

/// Put trashed items back where they came from. Returns how many came back.
pub fn restore_trashed(trash_paths: &[PathBuf]) -> usize {
    let mut back = 0;
    for entry in list_trash() {
        if trash_paths.contains(&entry.path) && restore_trash_entry(&entry).is_ok() {
            back += 1;
        }
    }
    back
}

/// Remove paths for good, folders and their contents included. Returns how many
/// went and the problems in words.
pub fn delete_permanently(paths: &[PathBuf]) -> (usize, Vec<String>) {
    let mut gone = 0;
    let mut problems = Vec::new();
    for path in paths {
        // A link to a folder is a file: removing it must not touch the folder it
        // points at.
        let linked = fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let outcome = if path.is_dir() && !linked {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        match outcome {
            Ok(()) => gone += 1,
            Err(e) => problems.push(format!(
                "{}: {}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                e
            )),
        }
    }
    (gone, problems)
}

pub fn list_trash() -> Vec<Entry> {
    let files = trash_root().join("files");
    let info = trash_root().join("info");
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(&files) else { return out };
    for item in rd.flatten() {
        let path = item.path();
        let mut entry = Entry::from_path(&path);
        let info_path = info.join(format!(
            "{}.trashinfo",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        if let Ok(text) = fs::read_to_string(&info_path) {
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("Path=") {
                    entry.original = Some(PathBuf::from(percent_decode(v.trim())));
                }
            }
        }
        out.push(entry);
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn empty_trash() -> Result<(), String> {
    let root = trash_root();
    for sub in ["files", "info"] {
        let dir = root.join(sub);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            let p = item.path();
            let res = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
            if let Err(e) = res {
                return Err(format!("{}: {}", p.display(), e));
            }
        }
    }
    Ok(())
}

pub fn restore_trash_entry(entry: &Entry) -> Result<(), String> {
    let Some(original) = &entry.original else {
        return Err("The former location of this item is unknown.".to_string());
    };
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut target = original.clone();
    let mut n = 1;
    while target.exists() {
        let stem = original
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = original
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        target = original.with_file_name(format!("{}-{}{}", stem, n, ext));
        n += 1;
    }
    fs::rename(&entry.path, &target).map_err(|e| e.to_string())?;
    let info = trash_root()
        .join("info")
        .join(format!("{}.trashinfo", entry.name));
    let _ = fs::remove_file(info);
    Ok(())
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    let mut an = String::new();
                    while let Some(c) = ai.peek().copied() {
                        if c.is_ascii_digit() {
                            an.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    let mut bn = String::new();
                    while let Some(c) = bi.peek().copied() {
                        if c.is_ascii_digit() {
                            bn.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    let av: u128 = an.trim_start_matches('0').parse().unwrap_or(0);
                    let bv: u128 = bn.trim_start_matches('0').parse().unwrap_or(0);
                    match av.cmp(&bv) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }
                } else {
                    let al = ac.to_lowercase().next().unwrap_or(ac);
                    let bl = bc.to_lowercase().next().unwrap_or(bc);
                    match al.cmp(&bl) {
                        std::cmp::Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

pub fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    natural_cmp(a, b)
}

pub fn format_duration(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 3600 {
        format!("{} min", (secs / 60).max(1))
    } else if secs < 86_400 {
        format!("{} h", secs / 3600)
    } else {
        format!("{} d", secs / 86_400)
    }
}

pub fn modified_within(entry: &Entry, hours: u64) -> bool {
    let Some(m) = entry.modified else { return false };
    let Ok(age) = SystemTime::now().duration_since(m) else {
        return true;
    };
    age.as_secs() <= hours * 3600
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn open_with_default(path: &Path) {
    let _ = std::process::Command::new("xdg-open")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn open_terminal(path: &Path) {
    for term in ["konsole", "x-terminal-emulator", "gnome-terminal", "alacritty", "xterm"] {
        let mut cmd = std::process::Command::new(term);
        if term == "konsole" {
            cmd.arg("--workdir").arg(path);
        } else if term == "gnome-terminal" {
            cmd.arg("--working-directory").arg(path);
        }
        if cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return;
        }
    }
}

/// Where the desktop keeps its list of recently used files: the freedesktop
/// `recently-used.xbel`. KIO's `recentlyused:` worker reads the same file
/// through `KRecentDocument`, which is why the original's recently-used view
/// and this section agree on what counts as recent.
pub fn recent_list_path() -> PathBuf {
    home_dir().join(".local/share/recently-used.xbel")
}

/// Added feature: the entries for the navigation panel's *Recent Files* group.
/// Newest first, at most `limit` rows, and only files that are still there —
/// the desktop's list keeps bookmarks for things that have since been moved or
/// deleted, and a row that opens nothing is worse than no row.
pub fn recent_files(limit: usize) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(recent_list_path()) else {
        return Vec::new();
    };
    parse_recent_xbel(&text, limit)
}

/// Read `recently-used.xbel` itself, so the same parse can be checked without
/// touching the person's own list. Each bookmark carries the file's address
/// (`href`), when it was last modified and when it was last visited; the
/// visit time is what "recently used" means, and the file's own modification
/// time is drawn from the file, not from the bookmark.
pub fn parse_recent_xbel(text: &str, limit: usize) -> Vec<Entry> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("<bookmark") {
        let after = &rest[start..];
        let Some(end) = after.find('>') else { break };
        let tag = &after[..end];
        rest = &after[end..];
        let Some(href) = attr(tag, "href") else { continue };
        let Some(path) = path_from_href(&href) else { continue };
        let when = attr(tag, "visited")
            .or_else(|| attr(tag, "modified"))
            .or_else(|| attr(tag, "added"))
            .and_then(|v| xbel_time(&v))
            .unwrap_or(UNIX_EPOCH);
        if !seen.insert(path.clone()) {
            continue;
        }
        found.push((when, path));
    }
    // A bookmark for a folder is still useful — it is a place the person was
    // working in — but the panel's Recent Files group is about files, so a
    // folder stays out of it and reaches the panel as a Recent Location.
    let mut out: Vec<Entry> = found
        .into_iter()
        .filter(|(_, p)| p.is_file())
        .map(|(when, p)| {
            let mut e = Entry::from_path(&p);
            e.modified = e.modified.or(Some(when));
            e
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(limit);
    out
}

/// One attribute out of an XML tag, `name="value"`. The list is small and
/// flat, so a scan beats pulling in a parser for it.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let at = tag.find(&needle)? + needle.len();
    let tail = &tag[at..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

/// `file:///home/me/a%20b.txt` -> `/home/me/a b.txt`. Anything that is not a
/// local file (`smb://`, `trash:/`) is refused, because the panel cannot open
/// it and a dead row helps nobody.
pub fn path_from_href(href: &str) -> Option<PathBuf> {
    let rest = href.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(percent_decode(rest)))
}

/// The bookmark's `visited` / `modified` stamp, as an instant. The list writes
/// `2026-11-06T18:41:00Z`; a stamp that will not parse is treated as old
/// rather than dropping the row.
pub fn xbel_time(value: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).into())
}

pub fn local_from_ymd(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> Option<SystemTime> {
    Local
        .with_ymd_and_hms(y, m, d, hh, mm, 0)
        .single()
        .map(|dt| dt.into())
}
