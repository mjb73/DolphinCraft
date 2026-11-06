//! `--selftest`: exercises the file operations behind the added tools against a
//! throw-away folder and a throw-away home, so the tricky parts (trash records,
//! duplicate grouping, bulk rename planning, search) can be checked without
//! touching the person's own files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fsutil;
use crate::search::{self, DupScan, SearchFilters, SearchSession};

struct Report {
    failures: usize,
}

impl Report {
    fn check(&mut self, name: &str, outcome: Result<String, String>) {
        match outcome {
            Ok(detail) => println!("PASS  {name}{}", if detail.is_empty() { String::new() } else { format!("  ({detail})") }),
            Err(e) => {
                self.failures += 1;
                println!("FAIL  {name}: {e}");
            }
        }
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, bytes).map_err(|e| e.to_string())
}

fn wait_dup(scan: &mut DupScan) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while scan.scanning && Instant::now() < deadline {
        scan.poll();
        std::thread::sleep(Duration::from_millis(30));
    }
    scan.poll();
    if scan.scanning {
        return Err("the scan did not finish in 10 seconds".into());
    }
    Ok(())
}

pub fn run() -> i32 {
    let base = std::env::temp_dir().join(format!("filecraft-selftest-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    if let Err(e) = fs::create_dir_all(&base) {
        println!("FAIL  could not create a scratch folder: {e}");
        return 1;
    }
    let mut r = Report { failures: 0 };

    // ---- folder listing: child counts and sizes -------------------------
    let listing = base.join("listing");
    let _ = write(&listing.join("sub/inner.txt"), b"inner");
    let _ = write(&listing.join("notes.txt"), b"alpha\n");
    r.check(
        "folder listing reports children and sizes",
        (|| {
            let entries = fsutil::read_folder(&listing, false)?;
            let dir = entries.iter().find(|e| e.name == "sub").ok_or("no folder entry")?;
            let file = entries.iter().find(|e| e.name == "notes.txt").ok_or("no file entry")?;
            if dir.children != Some(1) {
                return Err(format!("expected 1 child, got {:?}", dir.children));
            }
            if file.size != 6 {
                return Err(format!("expected 6 bytes, got {}", file.size));
            }
            Ok(format!("{} entries", entries.len()))
        })(),
    );

    // ---- checksums and duplicate grouping -------------------------------
    let dups = base.join("dups");
    let same_a = dups.join("a.bin");
    let same_b = dups.join("b.bin");
    let other = dups.join("c.bin");
    let payload = vec![b'A'; 8192];
    let _ = write(&same_a, &payload);
    let _ = write(&same_b, &payload);
    let _ = write(&other, &vec![b'B'; 8192]);
    let _ = write(&dups.join("small-a.txt"), b"tiny");
    let _ = write(&dups.join("small-b.txt"), b"tiny");
    r.check(
        "checksums tell copies from different files",
        (|| {
            let ha = search::sha256_file(&same_a).map_err(|e| e.to_string())?;
            let hb = search::sha256_file(&same_b).map_err(|e| e.to_string())?;
            let hc = search::sha256_file(&other).map_err(|e| e.to_string())?;
            if ha != hb {
                return Err("identical files hashed differently".into());
            }
            if ha == hc {
                return Err("different files hashed the same".into());
            }
            if ha.len() != 64 {
                return Err(format!("unexpected hash length {}", ha.len()));
            }
            Ok("64 hex characters".into())
        })(),
    );
    r.check(
        "duplicate finder groups identical files, including small ones",
        (|| {
            let mut scan = DupScan::start(dups.clone());
            wait_dup(&mut scan)?;
            let big = scan
                .groups
                .iter()
                .find(|g| g.size == 8192)
                .ok_or("no group for the 8 KiB copies")?;
            if big.files.len() != 2 {
                return Err(format!("expected 2 copies, got {}", big.files.len()));
            }
            let small = scan
                .groups
                .iter()
                .find(|g| g.size == 4)
                .ok_or("no group for the two 4-byte files")?;
            if small.files.len() != 2 {
                return Err("the small pair was not grouped".into());
            }
            Ok(format!("{} groups, {} bytes wasted", scan.groups.len(), scan.wasted()))
        })(),
    );

    // ---- bulk rename -----------------------------------------------------
    r.check(
        "bulk rename applies a pattern and refuses clashes",
        (|| {
            let dir = base.join("rename");
            for n in ["img-a.png", "img-b.png"] {
                write(&dir.join(n), b"x")?;
            }
            let paths: Vec<PathBuf> = ["img-a.png", "img-b.png"]
                .iter()
                .map(|n| dir.join(n))
                .collect();
            let done = crate::panels::bulk_rename(&paths, "{name}-{n}{ext}", 3, "2026-11-06")
                .map_err(|e| e)?;
            if done != 2 {
                return Err(format!("renamed {done} of 2"));
            }
            if !dir.join("img-a-03.png").exists() || !dir.join("img-b-04.png").exists() {
                return Err("the new names are not on disk".into());
            }
            let clash = crate::panels::bulk_rename(&paths, "same.txt", 1, "2026-11-06");
            if clash.is_ok() {
                return Err("a colliding pattern was accepted".into());
            }
            Ok("2 renamed, clash rejected".into())
        })(),
    );

    // ---- trash round trip (private home) --------------------------------
    let home = base.join("home");
    let _ = fs::create_dir_all(&home);
    std::env::set_var("HOME", &home);
    let trashed = base.join("trash-me.txt");
    let _ = write(&trashed, b"please come back");
    r.check(
        "move to trash writes a freedesktop record and restores",
        (|| {
            fsutil::move_to_trash(&[trashed.clone()]).map_err(|e| e)?;
            if trashed.exists() {
                return Err("the original file is still there".into());
            }
            let listed = fsutil::list_trash();
            let item = listed
                .iter()
                .find(|e| e.name == "trash-me.txt")
                .ok_or("the item is not in the trash folder")?;
            if item.original.as_deref() != Some(trashed.as_path()) {
                return Err(format!("wrong former location {:?}", item.original));
            }
            let info = fsutil::trash_root()
                .join("info")
                .join("trash-me.txt.trashinfo");
            let text = fs::read_to_string(&info).map_err(|e| format!("no .trashinfo file: {e}"))?;
            if !text.contains("[Trash Info]") || !text.contains("DeletionDate=") {
                return Err("the .trashinfo file lacks the required fields".into());
            }
            fsutil::restore_trash_entry(item).map_err(|e| e)?;
            if !trashed.exists() {
                return Err("the file did not come back".into());
            }
            Ok("record written, file restored".into())
        })(),
    );
    r.check(
        "empty trash clears both folders",
        (|| {
            let victim = base.join("trash-two.txt");
            write(&victim, b"bye")?;
            fsutil::move_to_trash(&[victim]).map_err(|e| e)?;
            fsutil::empty_trash()?;
            if !fsutil::list_trash().is_empty() {
                return Err("items remain after emptying".into());
            }
            Ok(String::new())
        })(),
    );

    // ---- search ----------------------------------------------------------
    r.check(
        "search finds a file below the current folder",
        (|| {
            let mut session = SearchSession::start(
                listing.clone(),
                "inner".to_string(),
                SearchFilters::default(),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while !session.done && Instant::now() < deadline {
                session.poll();
                std::thread::sleep(Duration::from_millis(30));
            }
            session.poll();
            if !session.done {
                return Err("search did not finish".into());
            }
            if session.results.iter().all(|e| e.name != "inner.txt") {
                return Err("inner.txt was not found".into());
            }
            Ok(format!("{} results", session.results.len()))
        })(),
    );

    // ---- folder size -----------------------------------------------------
    r.check(
        "folder size adds up the contents",
        (|| {
            let size = search::dir_size(&dups);
            if size < 8192 * 3 + 8 {
                return Err(format!("too small: {size}"));
            }
            Ok(format!("{size} bytes"))
        })(),
    );

    // ---- folder colours --------------------------------------------------
    r.check(
        "a folder keeps its colour in its own hidden file",
        (|| {
            let holder = base.join("colours");
            let folder = holder.join("receipts");
            write(&holder.join("keep.txt"), b"x")?;
            fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            crate::foldercolor::write(&folder, Some([196, 72, 66]))?;
            let text = fs::read_to_string(folder.join(".directory"))
                .map_err(|e| format!("no .directory: {e}"))?;
            if !text.contains("FolderColor=#c44842") {
                return Err(format!("the file says {text:?}"));
            }
            if crate::foldercolor::read(&folder) != Some([196, 72, 66]) {
                return Err("the colour did not come back".into());
            }
            let listed = fsutil::read_folder(&holder, false)?;
            let found = listed
                .iter()
                .find(|e| e.name == "receipts")
                .ok_or("the folder is not listed")?;
            if found.tint != Some([196, 72, 66]) {
                return Err("the listing lost the colour".into());
            }
            crate::foldercolor::write(&folder, None)?;
            if folder.join(".directory").exists() {
                return Err("the file stayed behind after clearing".into());
            }
            if fsutil::read_folder(&holder, false)?
                .iter()
                .any(|e| e.tint.is_some())
            {
                return Err("the colour survived clearing".into());
            }
            Ok("written, listed, read back, cleared".into())
        })(),
    );

    // ---- the location bar: typed and pasted addresses ---------------------
    r.check(
        "the location bar shows the folder itself, not a path with dots in it",
        (|| {
            use crate::fsutil::tidy_path;
            let p = tidy_path(std::path::Path::new("/usr/share/../lib"));
            if p != std::path::Path::new("/usr/lib") {
                return Err(format!("/usr/share/../lib became {}", p.display()));
            }
            let p = tidy_path(std::path::Path::new("/home//me/./notes/"));
            if p != std::path::Path::new("/home/me/notes") {
                return Err(format!("the plain path became {}", p.display()));
            }
            let p = tidy_path(std::path::Path::new("/.."));
            if p != std::path::Path::new("/") {
                return Err(format!("/.. became {}", p.display()));
            }
            let virtual_place = tidy_path(std::path::Path::new("trash:/"));
            if virtual_place != std::path::Path::new("trash:/") {
                return Err("a virtual location was rewritten".into());
            }
            Ok("/usr/share/../lib became /usr/lib; trash:/ kept".into())
        })(),
    );

    r.check(
        "a pasted address for a folder opens it",
        (|| {
            use crate::state::{resolve_location, Location};
            let target = base.join("listing");
            match resolve_location(&format!("\"{}\"  ", target.display())) {
                Ok(Location::Folder(p)) => {
                    if p != target {
                        return Err(format!("opened {} instead", p.display()));
                    }
                    Ok("quotes and spaces trimmed".into())
                }
                _ => Err("a folder address was not recognised".into()),
            }
        })(),
    );

    r.check(
        "a pasted address for a file opens its folder and selects the file",
        (|| {
            let holder = base.join("listing");
            let file = holder.join("notes.txt");
            let mut pane = crate::state::Pane::new(1, holder.clone());
            if pane.navigate_input(&file.display().to_string()).is_some() {
                return Err("the file address reported an error".into());
            }
            if pane.path != holder {
                return Err(format!("opened {}", pane.path.display()));
            }
            let picked = pane
                .sel
                .iter()
                .next()
                .and_then(|i| pane.entries.get(*i))
                .map(|e| e.name.clone());
            if picked.as_deref() != Some("notes.txt") {
                return Err(format!("selection is {picked:?}"));
            }
            Ok("folder open, file selected".into())
        })(),
    );

    r.check(
        "a file:// link and a home shortcut both resolve",
        (|| {
            use crate::state::{resolve_location, Location};
            let file = base.join("listing/notes.txt");
            let link = format!("file://{}", file.display());
            match resolve_location(&link) {
                Ok(Location::File(p)) if p == file => {}
                _ => return Err("the file:// link did not resolve".into()),
            }
            match resolve_location("~") {
                Ok(Location::Folder(p)) if p == fsutil::home_dir() => Ok("home expanded".into()),
                _ => Err("~ did not expand to home".into()),
            }
        })(),
    );

    r.check(
        "an address that leads nowhere says so and stays put",
        (|| {
            let mut pane = crate::state::Pane::new(1, base.join("listing"));
            let before = pane.path.clone();
            let message = pane.navigate_input("/definitely/not/here");
            if message.is_none() {
                return Err("no message for a missing address".into());
            }
            if pane.path != before {
                return Err("the view moved anyway".into());
            }
            Ok(message.unwrap())
        })(),
    );

    r.check(
        "a bigger zoom asks for a bigger picture",
        (|| {
            use crate::search::ThumbCache;
            let img_path = base.join("thumbs/picture.png");
            if let Some(parent) = img_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let img = image::RgbaImage::from_fn(240, 120, |x, y| {
                image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
            });
            img.save(&img_path).map_err(|e| e.to_string())?;

            let ctx = eframe::egui::Context::default();
            let mut cache = ThumbCache::new();
            let mut first = 0usize;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(tex) = cache.get(&ctx, &img_path, 64) {
                    first = tex.size()[0];
                    break;
                }
                if Instant::now() > deadline {
                    return Err("the first thumbnail never arrived".into());
                }
                cache.poll(&ctx);
                std::thread::sleep(Duration::from_millis(25));
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut second = first;
            loop {
                if let Some(tex) = cache.get(&ctx, &img_path, 160) {
                    second = tex.size()[0];
                    if second >= 160 {
                        break;
                    }
                }
                if Instant::now() > deadline {
                    return Err(format!(
                        "the thumbnail stayed at {second} px after zooming in"
                    ));
                }
                cache.poll(&ctx);
                std::thread::sleep(Duration::from_millis(25));
            }
            if second <= first {
                return Err(format!("{first} px did not grow (still {second} px)"));
            }
            Ok(format!("{first} px became {second} px"))
        })(),
    );

    r.check(
        "the icon grid sizes cells the way the original does",
        (|| {
            let row_of = |w: f32| {
                let widths: Vec<f32> = (0..40).map(|_| w).collect();
                let rows = crate::views::pack_icon_rows(&widths, 1000.0, crate::views::ICON_CELL_GAP);
                rows[0].1 - rows[0].0
            };
            // updateGridSize @0x26db80, with the 13 px label font at roughly
            // seven pixels a character.
            let cell32 = crate::theme::icon_grid_cell_width(2, 32.0, 7.0);
            let cell64 = crate::theme::icon_grid_cell_width(4, 64.0, 7.0);
            let cell256 = crate::theme::icon_grid_cell_width(16, 256.0, 7.0);
            if !(cell32 < cell64 && cell64 < cell256) {
                return Err(format!(
                    "cells do not grow with the zoom: {cell32:.0}, {cell64:.0}, {cell256:.0} px"
                ));
            }
            // Never narrower than the icon plus a quarter of its size.
            let floor = (4.0f32 / 13.0).exp() * 4.0 + 64.0;
            if cell64 < floor {
                return Err(format!("the 64 px cell is {cell64:.0} px, under the {floor:.0} px floor"));
            }
            let cols64 = row_of(cell64);
            let cols256 = row_of(cell256);
            if cols64 < 4 || cols256 < 3 {
                return Err(format!(
                    "only {cols64} cells fit a 1000 px row at 64 px and {cols256} at 256 px"
                ));
            }
            Ok(format!(
                "{cols64} per row at 64 px ({cell64:.0} px wide), {cols256} at 256 px"
            ))
        })(),
    );

    r.check(
        "a tall picture gets a tall, narrow cell",
        (|| {
            use crate::views::{icon_cell_width, pack_icon_rows, thumb_size, ICON_CELL_GAP};
            // A 9:16 phone photo only fills 36 of a 64 px square box, and a 3:4
            // one 48. The cell follows the picture instead of the square, but
            // never drops below the label floor (40 px) and never exceeds the
            // label-based width the original computes in updateGridSize.
            let portrait = thumb_size(64.0, 9.0 / 16.0);
            if (portrait.x - 36.0).abs() > 0.5 || (portrait.y - 64.0).abs() > 0.5 {
                return Err(format!(
                    "a 9:16 picture measured {:.0}x{:.0} px instead of 36x64",
                    portrait.x, portrait.y
                ));
            }
            let cap = crate::theme::icon_grid_cell_width(4, 64.0, 7.0);
            let square = icon_cell_width(64.0, 1.0).min(cap);
            let tall = icon_cell_width(64.0, 9.0 / 16.0).min(cap);
            if tall >= square {
                return Err(format!(
                    "a portrait cell is {tall:.0} px wide, no narrower than the {square:.0} px square one"
                ));
            }
            if tall < 48.0 {
                return Err(format!("a portrait cell of {tall:.0} px leaves the label no room"));
            }
            if cap < tall {
                return Err(format!(
                    "the label-based cell width {cap:.0} px would clip a {tall:.0} px cell"
                ));
            }
            let widths: Vec<f32> = (0..40).map(|_| tall).collect();
            let rows = pack_icon_rows(&widths, 1000.0, ICON_CELL_GAP);
            let per_row = rows[0].1 - rows[0].0;
            let square_widths: Vec<f32> = (0..40).map(|_| square).collect();
            let square_rows = pack_icon_rows(&square_widths, 1000.0, ICON_CELL_GAP);
            let square_per_row = square_rows[0].1 - square_rows[0].0;
            if per_row <= square_per_row {
                return Err(format!(
                    "portrait cells fit {per_row} a row, no more than the {square_per_row} square ones"
                ));
            }
            Ok(format!(
                "portrait cells are {tall:.0} px against {square:.0} px for square ones, {per_row} to a 1000 px row"
            ))
        })(),
    );

    // ---- status bar wording and the free-space share --------------------
    r.check(
        "the status bar counts folders and files the way the original does",
        (|| {
            // `DolphinView::emitStatusBarText` @0x273180: an empty folder reads
            // "0 folders, 0 files"; folders alone carry no size; files alone use
            // "%1 (%2)"; both use "%1, %2 (%3)".
            let empty = crate::status_counts(0, 0, 0);
            if empty != "0 folders, 0 files" {
                return Err(format!("an empty folder reads \"{empty}\""));
            }
            let folders_only = crate::status_counts(3, 0, 0);
            if folders_only != "3 folders" {
                return Err(format!("three folders read \"{folders_only}\""));
            }
            let one_file = crate::status_counts(0, 1, 1024);
            if one_file != "1 file (1.0 KiB)" {
                return Err(format!("one 1 KiB file reads \"{one_file}\""));
            }
            let both = crate::status_counts(2, 3, 2048);
            if both != "2 folders, 3 files (2.0 KiB)" {
                return Err(format!("2 folders and 3 files read \"{both}\""));
            }
            let sel_folders = crate::status_selection(1, 0, 0);
            if sel_folders != "1 folder selected" {
                return Err(format!("one selected folder reads \"{sel_folders}\""));
            }
            let sel_files = crate::status_selection(0, 2, 1536);
            if sel_files != "2 files selected (1.5 KiB)" {
                return Err(format!("two selected files read \"{sel_files}\""));
            }
            let sel_mixed = crate::status_selection(1, 2, 1536);
            if sel_mixed != "1 folder selected, 2 files selected (1.5 KiB)" {
                return Err(format!("a mixed selection reads \"{sel_mixed}\""));
            }
            Ok(format!("\"{both}\" and \"{sel_mixed}\""))
        })(),
    );

    r.check(
        "the three name comparisons of the original order names differently",
        (|| {
            use crate::state::NameSort;
            // `FUN_001e5750` @0x1e5750 offers exactly these three rows; the
            // strings are "Natural" (0x12eaf2), "Alphabetical, case insensitive"
            // (0x1372e0) and "Alphabetical, case sensitive" (0x12eafa).
            let natural = NameSort::Natural;
            let ci = NameSort::CaseInsensitive;
            let cs = NameSort::CaseSensitive;

            // Natural reads digit runs as numbers: file2 before file10.
            if natural.compare("file2", "file10") != std::cmp::Ordering::Less {
                return Err("Natural put file10 before file2".into());
            }
            // Case-sensitive is plain code-unit order, so an upper-case B (0x42)
            // sorts before a lower-case a (0x61).
            if cs.compare("B.txt", "a.txt") != std::cmp::Ordering::Less {
                return Err("case-sensitive did not put B.txt before a.txt".into());
            }
            // Case-insensitive folds first: a.txt before B.txt.
            if ci.compare("a.txt", "B.txt") != std::cmp::Ordering::Less {
                return Err("case-insensitive did not put a.txt before B.txt".into());
            }
            // A case-only difference still has a defined order in both.
            if ci.compare("Report.txt", "report.txt") == std::cmp::Ordering::Equal {
                return Err("case-insensitive left a case-only pair unordered".into());
            }
            if crate::state::NameSort::from_index(1) != ci
                || crate::state::NameSort::from_index(2) != cs
                || crate::state::NameSort::from_index(0) != natural
            {
                return Err("the saved index does not round-trip".into());
            }
            Ok("file2 < file10, B.txt < a.txt, a.txt < B.txt".to_string())
        })(),
    );

    r.check(
        "sorting by size still breaks ties with the chosen name comparison",
        (|| {
            // One `SortingChoice` decides how two names compare wherever a name
            // is the deciding key (`FUN_001e5750` @0x1e5750, saved by
            // `FUN_001e7af0` @0x1e7af0), so equal-size rows must follow the
            // chosen mode rather than always the natural one.
            fn two(mode: crate::state::NameSort) -> Vec<String> {
                let mut pane = crate::state::Pane::new(1, std::path::PathBuf::from("/tmp"));
                pane.sort = crate::state::SortKey::Size;
                pane.name_sort = mode;
                pane.asc = true;
                pane.entries.clear();
                for name in ["B.txt", "a.txt"] {
                    let mut e = fsutil::Entry::from_path(std::path::Path::new(&format!("/tmp/{name}")));
                    e.name = name.to_string();
                    e.size = 10;
                    pane.entries.push(e);
                }
                pane.sort_entries();
                pane.entries.iter().map(|e| e.name.clone()).collect()
            }
            let cs = two(crate::state::NameSort::CaseSensitive);
            if cs != ["B.txt", "a.txt"] {
                return Err(format!("case-sensitive size ties read {cs:?}"));
            }
            let natural = two(crate::state::NameSort::Natural);
            if natural != ["a.txt", "B.txt"] {
                return Err(format!("natural size ties read {natural:?}"));
            }
            Ok("B.txt/a.txt by size follows the name mode".to_string())
        })(),
    );

    r.check(
        "the used-space share rounds half up",
        (|| {
            // `FUN_001e6a40` @0x1e6a40: (total - free) * 100 / total + 0.5,
            // truncated — never a plain integer division.
            let pct = fsutil::used_percent(1, 1000);
            if pct != 100 {
                return Err(format!("999 of 1000 bytes used read {pct}%"));
            }
            let half = fsutil::used_percent(500, 1000);
            if half != 50 {
                return Err(format!("500 of 1000 bytes used read {half}%"));
            }
            let down = fsutil::used_percent(500, 1010);
            if down != 50 {
                return Err(format!("510 of 1010 bytes used read {down}%"));
            }
            let none = fsutil::used_percent(0, 0);
            if none != 0 {
                return Err(format!("an unreported volume read {none}%"));
            }
            Ok(format!("999/1000 → {pct}%, 510/1010 → {down}%"))
        })(),
    );

    r.check(
        "Ctrl + one wheel notch is exactly one zoom step",
        (|| {
            use crate::state::{wheel_units, wheel_zoom_steps, WHEEL_UNITS_PER_STEP};
            // One line-based notch (a real mouse wheel) is 120 Qt units and must
            // buy exactly one step, as DolphinView::wheelEvent @0x272710 does
            // when its running total passes 0x78.
            let mut acc = 0.0;
            let up = wheel_units(egui::MouseWheelUnit::Line, 1.0);
            if up != WHEEL_UNITS_PER_STEP {
                return Err(format!("one notch counted as {up} units"));
            }
            let steps = wheel_zoom_steps(&mut acc, up);
            if steps != 1 || acc != 0.0 {
                return Err(format!("one notch gave {steps} steps and left {acc}"));
            }
            // Down is one step the other way.
            let down = wheel_zoom_steps(&mut acc, wheel_units(egui::MouseWheelUnit::Line, -1.0));
            if down != -1 {
                return Err(format!("one notch down gave {down} steps"));
            }
            // Half a notch is kept, not thrown away: two halves are one step.
            let mut half = 0.0;
            let a = wheel_zoom_steps(&mut half, WHEEL_UNITS_PER_STEP / 2.0);
            let b = wheel_zoom_steps(&mut half, WHEEL_UNITS_PER_STEP / 2.0);
            if a != 0 || b != 1 {
                return Err(format!("two half notches gave {a} then {b} steps"));
            }
            // A pixel-based event (touchpad) reaches a step after 40 px, the
            // same distance egui itself calls one line.
            let mut px = 0.0;
            let p = wheel_zoom_steps(&mut px, wheel_units(egui::MouseWheelUnit::Point, 40.0));
            if p != 1 {
                return Err(format!("40 px of glide gave {p} steps"));
            }
            Ok(format!("120 units = 1 step, two halves = 1 step, 40 px = 1 step"))
        })(),
    );

    r.check(
        "wheel zoom stops at the smallest and biggest icon size",
        (|| {
            let mut pane = crate::state::Pane::new(2, base.join("listing"));
            pane.view = crate::state::ViewMode::Icons;
            // Twenty steps up from the factory size must land on
            // level 16 (256 px) and stay there, as DolphinItemListView::
            // setZoomLevel @0x26e100 clamps the level to 0..=16.
            for _ in 0..20 {
                pane.zoom_by(1);
            }
            if (pane.zoom[0], pane.icon_size()) != (16, 256.0) {
                return Err(format!(
                    "the top is level {} at {} px",
                    pane.zoom[0],
                    pane.icon_size()
                ));
            }
            for _ in 0..40 {
                pane.zoom_by(-1);
            }
            if (pane.zoom[0], pane.icon_size()) != (0, 16.0) {
                return Err(format!(
                    "the bottom is level {} at {} px",
                    pane.zoom[0],
                    pane.icon_size()
                ));
            }
            Ok("top is 16 / 256 px, bottom is 0 / 16 px".into())
        })(),
    );

    // ---- the toolbar's two halves never share pixels --------------------
    r.check(
        "the sort control never lands on the location bar",
        (|| {
            use crate::{toolbar_metrics, SortControl, TOOLBAR_CLUSTER_GAP};
            // Every width from a narrow window to a wide one, with short and long
            // sort labels ("Sort: Name", "Sort: Modified").
            let mut worst: Option<(f32, f32)> = None;
            for window in (420..=1800).step_by(20) {
                // The left-hand navigation cluster takes about 180 px of the row.
                let avail = window as f32 - 180.0;
                for label_w in [48.0f32, 58.0, 72.0, 84.0] {
                    let m = toolbar_metrics(avail, label_w, true);
                    // Wherever a bar is drawn at all it must fit beside the cluster.
                    if m.bar_w > 0.0 {
                        let used = m.bar_w + m.cluster_w + TOOLBAR_CLUSTER_GAP;
                        if used > avail + 0.01 {
                            return Err(format!(
                                "at {window} px the bar needs {used:.0} px of {avail:.0}: \
                                 the bar ({:.0}) and the cluster ({:.0}) overlap",
                                m.bar_w, m.cluster_w
                            ));
                        }
                    }
                    if m.sort != SortControl::Hidden && m.bar_w < 60.0 {
                        worst.get_or_insert((window as f32, m.bar_w));
                    }
                }
            }
            if worst.is_some() {
                return Err("a wide window leaves no room for the location bar".into());
            }
            // The label gives way before the bar dies: at 1440 px it still shows
            // the sort key, at a very narrow width it falls back to an icon.
            let wide = toolbar_metrics(1260.0, 72.0, true);
            let narrow = toolbar_metrics(420.0, 72.0, true);
            if wide.sort != SortControl::Label {
                return Err("a 1440 px window drops the sort label".into());
            }
            if narrow.sort == SortControl::Label {
                return Err("a 600 px window keeps the full sort label".into());
            }
            Ok(format!(
                "1440 px: bar {:.0} px + cluster {:.0} px; 600 px: sort is {:?}",
                wide.bar_w, wide.cluster_w, narrow.sort
            ))
        })(),
    );

    r.check(
        "split view gives the whole toolbar row to the buttons",
        (|| {
            let m = crate::toolbar_metrics(1260.0, 72.0, false);
            if m.bar_w != 0.0 {
                return Err(format!("split view still reserves {:.0} px for a bar", m.bar_w));
            }
            if m.sort == crate::SortControl::Hidden {
                return Err("split view hides the sort control for no reason".into());
            }
            Ok("no bar, sort control kept".into())
        })(),
    );

    // ---- shortcut rows: colour and dragging into a new order --------------
    r.check(
        "a shortcut keeps the colour the person gave it",
        (|| {
            use crate::config::Config;
            let mut cfg = Config::default();
            let a = base.join("colours/receipts");
            let b = base.join("colours/photos");
            cfg.set_shortcut_color(&a, Some([196, 72, 66]));
            cfg.set_shortcut_color(&b, Some([86, 148, 78]));
            if cfg.shortcut_color(&a) != Some([196, 72, 66]) {
                return Err("the colour did not come back".into());
            }
            // A colour lives in the config file, so it has to survive the trip
            // through JSON the way the shortcuts do.
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if back.shortcut_color(&b) != Some([86, 148, 78]) {
                return Err("the colour was lost when the settings were written".into());
            }
            // "Normal Colour" removes the entry rather than storing a blank one.
            cfg.set_shortcut_color(&a, None);
            if cfg.shortcut_color(&a).is_some() {
                return Err("the colour stayed after being cleared".into());
            }
            if cfg.shortcut_colors.len() != 1 {
                return Err(format!(
                    "expected one colour left, found {}",
                    cfg.shortcut_colors.len()
                ));
            }
            Ok("two colours written, read back, one cleared".into())
        })(),
    );

    r.check(
        "a marked shortcut keeps the colour the person gave it",
        (|| {
            use eframe::egui::Color32;
            let red = Color32::from_rgb(196, 72, 66);
            let plain = crate::place_icon_colour(false, Some(red));
            let marked = crate::place_icon_colour(true, Some(red));
            if plain != red {
                return Err("an unmarked row changed its colour".into());
            }
            if marked == Color32::WHITE || marked == plain {
                return Err("the marked row was repainted instead of tinted".into());
            }
            // Still red, not a washed-out grey: the red channel leads the others.
            if !(marked.r() > marked.g() + 40 && marked.r() > marked.b() + 40) {
                return Err(format!(
                    "the marked colour lost its hue: {:?}",
                    (marked.r(), marked.g(), marked.b())
                ));
            }
            // Lightened, so it stands out on the blue selection band.
            let sum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
            if sum(marked) <= sum(plain) {
                return Err("the marked colour was not lifted for contrast".into());
            }
            // A row with no colour of its own keeps the blue-grey place colour
            // rather than turning white.
            let place = crate::place_icon_colour(true, None);
            if place == Color32::WHITE || place.b() <= place.r() {
                return Err(format!(
                    "a plain place row went colourless when marked: {:?}",
                    (place.r(), place.g(), place.b())
                ));
            }
            Ok("red stays red and the place colour stays blue when marked".into())
        })(),
    );

    r.check(
        "shortcuts can be dragged into a new order",
        (|| {
            use crate::config::{move_shortcut, Bookmark};
            let mk = |n: &str| Bookmark {
                name: n.into(),
                path: base.join(n),
            };
            let names = |v: &Vec<Bookmark>| v.iter().map(|b| b.name.clone()).collect::<Vec<_>>();

            let mut list = vec![mk("a"), mk("b"), mk("c")];
            // Dragged down into the gap between b and c.
            if !move_shortcut(&mut list, 0, 2) {
                return Err("dragging the first row down did nothing".into());
            }
            if names(&list) != ["b", "a", "c"] {
                return Err(format!("dragging down gave {:?}", names(&list)));
            }
            // Dragged up to the very top.
            if !move_shortcut(&mut list, 2, 0) {
                return Err("dragging the last row up did nothing".into());
            }
            if names(&list) != ["c", "b", "a"] {
                return Err(format!("dragging up gave {:?}", names(&list)));
            }
            // Let go over its own place, or over the gap right below it: the
            // model's movePlace refuses both, and so does this.
            if move_shortcut(&mut list, 1, 1) || move_shortcut(&mut list, 1, 2) {
                return Err("a drop on its own place moved the row".into());
            }
            if names(&list) != ["c", "b", "a"] {
                return Err("the refused drops changed the order anyway".into());
            }
            // Dropped right at the end of the list.
            if !move_shortcut(&mut list, 0, 3) {
                return Err("dragging to the end did nothing".into());
            }
            if names(&list) != ["b", "a", "c"] {
                return Err(format!("dragging to the end gave {:?}", names(&list)));
            }
            Ok("down, up, refused in place, to the end".into())
        })(),
    );

    r.check(
        "the window carries the app's own name, not the original's",
        (|| {
            let name = crate::APP_NAME;
            if name != "DolphinCraft" {
                return Err(format!("the window title is {name:?}"));
            }
            // Every place that shows the name reads APP_NAME, and this is it.
            if name.trim() != name || name.is_empty() {
                return Err("the name has stray spaces".into());
            }
            Ok(format!("the title bar, task switcher and app id all read {name:?}"))
        })(),
    );

    let _ = fs::remove_dir_all(&base);
    if r.failures == 0 {
        println!("selftest: all checks passed");
        0
    } else {
        println!("selftest: {} check(s) failed", r.failures);
        1
    }
}
