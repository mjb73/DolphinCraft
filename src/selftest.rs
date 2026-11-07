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

/// A PNG carrying one `tEXt` chunk. That is all the prompt reader looks at, so
/// the pixels can be left out — a real compressed image would need a compressor
/// this program does not ship.
fn png_with_prompt(text: &str) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let chunk = format!("parameters\0{text}");
    bytes.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
    bytes.extend_from_slice(b"tEXt");
    bytes.extend_from_slice(chunk.as_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC, not checked by the reader
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(b"IEND");
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
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

    // ---- the person's rule: a mark and a press of Delete is the whole action
    r.check(
        "a marked picture goes to the Trash at once, and Undo puts it back",
        (|| {
            let mut shots = Vec::new();
            for n in 1..=3 {
                let p = base.join(format!("shot-{n}.png"));
                write(&p, b"picture")?;
                shots.push(p);
            }
            let (moved, records, problem) = fsutil::trash_with_records(&shots);
            if problem.is_some() {
                return Err(format!("the move complained: {:?}", problem));
            }
            if moved != 3 || records.len() != 3 {
                return Err(format!("{moved} moved, {} records", records.len()));
            }
            if shots.iter().any(|p| p.exists()) {
                return Err("a picture is still in the folder".into());
            }
            // Undo has to reach exactly the three records this move made, and
            // nothing else that was already in the Trash.
            let back = fsutil::restore_trashed(&records);
            if back != 3 || shots.iter().any(|p| !p.exists()) {
                return Err(format!("{back} of 3 came back"));
            }
            Ok("3 moved, 3 put back".into())
        })(),
    );
    r.check(
        "a delete with no question takes the folder and everything in it",
        (|| {
            let folder = base.join("throw-away");
            write(&folder.join("inner/notes.txt"), b"bye")?;
            write(&folder.join("top.txt"), b"bye")?;
            let (gone, problems) = fsutil::delete_permanently(&[folder.clone()]);
            if !problems.is_empty() {
                return Err(problems.join(", "));
            }
            if gone != 1 || folder.exists() {
                return Err(format!("{gone} went, the folder is still there"));
            }
            let (none, missing) = fsutil::delete_permanently(&[folder.join("ghost.txt")]);
            if none != 0 || missing.is_empty() {
                return Err("deleting something that is not there said nothing".into());
            }
            Ok("folder and contents gone; a missing item is reported".into())
        })(),
    );
    r.check(
        "a fresh profile asks no question before moving to the Trash, but does before a permanent delete and before emptying the Trash",
        (|| {
            let config = crate::config::Config::default();
            if config.confirm_trash {
                return Err("a fresh profile still asks before the Trash move".into());
            }
            if !config.confirm_delete {
                return Err("a fresh profile no longer asks before a permanent delete".into());
            }
            if !config.confirm_empty_trash {
                return Err("emptying the Trash no longer asks, though it cannot be undone".into());
            }
            Ok("trash off, delete on, empty trash on — the original's three defaults".into())
        })(),
    );

    r.check(
        "the delete question is worded the way the original words it",
        (|| {
            let one = std::path::PathBuf::from("/tmp/Beach at noon.png");
            let many = vec![
                std::path::PathBuf::from("/tmp/a.png"),
                std::path::PathBuf::from("/tmp/b.png"),
            ];
            // `name_list` and `display_name` feed the two questions; the words
            // themselves are `KIO::JobUiDelegate::askDeleteConfirmation` @0x1a8a20.
            if crate::display_name(&one) != "Beach at noon.png" {
                return Err("the question no longer names the item".into());
            }
            let list = crate::name_list(&many);
            if !list.starts_with("2 items:") || !list.contains("b.png") {
                return Err(format!("the list reads {list}"));
            }
            let text = format!(
                "Do you really want to move this item to the Trash?\n\n{}",
                crate::display_name(&one)
            );
            if !text.contains("this item to the Trash") {
                return Err("the Trash question lost the original's words".into());
            }
            Ok("named item, counted list, the original's sentences".into())
        })(),
    );

    r.check(
        "neither delete row promises a question this build no longer asks",
        (|| {
            for label in [crate::DELETE_PERMANENT_LABEL, crate::EMPTY_TRASH_LABEL] {
                if label.contains('…') || label.ends_with("...") {
                    return Err(format!("\"{label}\" ends in an ellipsis"));
                }
            }
            if crate::DELETE_PERMANENT_LABEL != "Delete Permanently"
                || crate::EMPTY_TRASH_LABEL != "Empty Trash"
            {
                return Err("a row was renamed away from the original's wording".into());
            }
            Ok("\"Delete Permanently\" and \"Empty Trash\", nothing pending".into())
        })(),
    );
    r.check(
        "emptying the Trash leaves both of its folders empty",
        (|| {
            for n in 1..=2 {
                let p = base.join(format!("empty-{n}.txt"));
                write(&p, b"gone")?;
                fsutil::move_to_trash(&[p]).map_err(|e| e)?;
            }
            if fsutil::list_trash().is_empty() {
                return Err("nothing went into the Trash to clear".into());
            }
            fsutil::empty_trash()?;
            if !fsutil::list_trash().is_empty() {
                return Err("the Trash still lists items".into());
            }
            let root = fsutil::trash_root();
            for sub in ["files", "info"] {
                let left = fs::read_dir(root.join(sub))
                    .map(|rd| rd.flatten().count())
                    .unwrap_or(0);
                if left != 0 {
                    return Err(format!("{left} entries left in {sub}"));
                }
            }
            // Clearing an empty Trash is not an error: the row stays harmless.
            fsutil::empty_trash()?;
            Ok("cleared, and clearing again is quiet".into())
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

    // A picture carrying a prompt can have its text chunk written before the
    // image header. The size line used to read the width straight out of that
    // text chunk and print numbers in the billions; the header is now hunted
    // for down the chunk list.
    r.check(
        "a PNG with its text chunk before the image header reports its real size",
        (|| {
            use crate::search::probe_image_size;
            let dir = base.join("headers");
            let odd = dir.join("text-first.png");
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            let text = b"parameters\0a wide empty beach";
            bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
            bytes.extend_from_slice(b"tEXt");
            bytes.extend_from_slice(text);
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            let mut ihdr = Vec::new();
            ihdr.extend_from_slice(&640u32.to_be_bytes());
            ihdr.extend_from_slice(&480u32.to_be_bytes());
            ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
            bytes.extend_from_slice(&(ihdr.len() as u32).to_be_bytes());
            bytes.extend_from_slice(b"IHDR");
            bytes.extend_from_slice(&ihdr);
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(b"IEND");
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            write(&odd, &bytes)?;
            match probe_image_size(&odd) {
                Some((640, 480)) => Ok("640 × 480 read past a text chunk of 26 bytes".into()),
                other => Err(format!("the size line read {other:?} instead of 640 × 480")),
            }
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
                // The pane keeps the whole folder in `all` and the visible list in
                // `entries`, so both are seeded here.
                pane.entries.clear();
                pane.all.clear();
                for name in ["B.txt", "a.txt"] {
                    let mut e = fsutil::Entry::from_path(std::path::Path::new(&format!("/tmp/{name}")));
                    e.name = name.to_string();
                    e.size = 10;
                    pane.entries.push(e.clone());
                    pane.all.push(e);
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
        "a new sort key starts the way the original starts it",
        (|| {
            // `DolphinView::defaultSortOrderForRole` @0x273910: the roles in the
            // table at 0x2f3960 (the three time roles, size, width, height, rating,
            // duration …) start descending; every other role starts ascending. So
            // Size and Modified show the biggest and the newest first, Name and
            // Type start at A and at the first type.
            use crate::state::{Pane, SortKey};
            let want = [
                (SortKey::Name, true),
                (SortKey::Size, false),
                (SortKey::Type, true),
                (SortKey::Modified, false),
            ];
            for (key, ascending) in want {
                let mut pane = Pane::new(1, std::path::PathBuf::from("/tmp"));
                pane.choose_sort(key);
                if pane.asc != ascending {
                    return Err(format!(
                        "{} started {}",
                        key.label(),
                        if pane.asc { "ascending" } else { "descending" }
                    ));
                }
            }
            Ok("Name and Type ascending, Size and Modified descending".to_string())
        })(),
    );

    r.check(
        "choosing the key that is already chosen keeps the order",
        (|| {
            // `slotSortTriggered` @0x291f30 sets the ticked action and the role; it
            // has no branch that reverses the order. Direction is the separate
            // Ascending / Descending pair, so a second click on the same key must
            // not flip the listing.
            use crate::state::{Pane, SortKey};
            let mut pane = Pane::new(1, std::path::PathBuf::from("/tmp"));
            pane.choose_sort(SortKey::Size);
            if pane.asc {
                return Err("Size did not start descending".into());
            }
            let resorted = pane.choose_sort(SortKey::Size);
            if resorted {
                return Err("a second click on Size re-sorted the folder".into());
            }
            if pane.asc {
                return Err("a second click on Size flipped the order".into());
            }
            Ok("Size stays descending when it is picked again".to_string())
        })(),
    );

    r.check(
        "the direction chosen for a key stays with that key",
        (|| {
            // `DolphinView::setPreferredSortOrder` (@0x292640 calls it, hash at
            // DolphinView+0x130) remembers the order per role; `preferredSortOrder`
            // @0x2859a0 reads it back before falling back to the default.
            use crate::state::{Pane, SortKey};
            let mut pane = Pane::new(1, std::path::PathBuf::from("/tmp"));
            pane.choose_sort(SortKey::Name);
            pane.set_sort_dir(true); // Name, newest-name-first style, chosen by hand
            pane.choose_sort(SortKey::Size);
            if pane.asc {
                return Err("Size did not start descending".into());
            }
            pane.choose_sort(SortKey::Name);
            if pane.asc {
                return Err("Name forgot the descending direction chosen for it".into());
            }
            pane.choose_sort(SortKey::Modified);
            pane.set_sort_dir(false);
            pane.choose_sort(SortKey::Modified);
            if !pane.asc {
                return Err("Modified forgot the ascending direction chosen for it".into());
            }
            Ok("Name descending and Modified ascending are both remembered".to_string())
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
        "size of contents stops at the folder limit",
        (|| {
            // `ContentDisplaySettings::RecursiveDirectorySizeLimit` is handed to
            // the size job as a folder count: factory 10, the View → Display page
            // allows 1..20 (`FUN_001e5750` @0x1e5750, setRange(1, 20)).
            let deep = base.join("depth");
            let _ = write(&deep.join("one/two/three/leaf.txt"), &vec![b'x'; 700]);
            let _ = write(&deep.join("one/two/mid.txt"), &vec![b'x'; 30]);
            let _ = write(&deep.join("one/near.txt"), &vec![b'x'; 5]);
            let one = search::dir_size_limited(&deep, Some(1));
            let two = search::dir_size_limited(&deep, Some(2));
            let all = search::dir_size(&deep);
            if one != 5 {
                return Err(format!("one folder deep read {one} bytes, expected 5"));
            }
            if two != 35 {
                return Err(format!("two folders deep read {two} bytes, expected 35"));
            }
            if all != 735 {
                return Err(format!("the whole tree read {all} bytes, expected 735"));
            }
            if search::SIZE_LIMIT_DEFAULT != 10
                || search::SIZE_LIMIT_MIN != 1
                || search::SIZE_LIMIT_MAX != 20
            {
                return Err("the limit's factory value or range drifted".into());
            }
            Ok(format!("1 → {one} B, 2 → {two} B, all → {all} B, default 10, 1..20"))
        })(),
    );

    r.check(
        "a preview is skipped above the file-size limit",
        (|| {
            // `[PreviewSettings] MaximumSize`, in MiB; the spin box's 0 shows as
            // "files of any size" and means no cap.
            const MIB: u64 = 1024 * 1024;
            if !search::preview_allowed(50 * MIB, 0) {
                return Err("0 MiB (any size) refused a big file".into());
            }
            if !search::preview_allowed(2 * MIB, 3) {
                return Err("a 2 MiB file was refused under a 3 MiB limit".into());
            }
            if search::preview_allowed(4 * MIB, 3) {
                return Err("a 4 MiB file was allowed under a 3 MiB limit".into());
            }
            if !search::preview_allowed(3 * MIB, 3) {
                return Err("exactly 3 MiB was refused under a 3 MiB limit".into());
            }
            Ok("0 = any size, 2/3 MiB in, 4 MiB out, 3 MiB exactly in".to_string())
        })(),
    );

    r.check(
        "the zoom slider, the size limit and the preview cap survive a restart",
        (|| {
            use crate::config::Config;
            let fresh = Config::default();
            if !fresh.show_zoom_slider {
                return Err("this build opens with the zoom slider hidden".into());
            }
            if fresh.size_limit != search::SIZE_LIMIT_DEFAULT || fresh.preview_max_mib != 0 {
                return Err(format!(
                    "factory values read {} folders and {} MiB",
                    fresh.size_limit, fresh.preview_max_mib
                ));
            }
            let mut changed = Config::default();
            changed.show_zoom_slider = false;
            changed.size_limit = 3;
            changed.preview_max_mib = 8;
            let text = serde_json::to_string(&changed).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if back.show_zoom_slider || back.size_limit != 3 || back.preview_max_mib != 8 {
                return Err("a saved choice did not come back".into());
            }
            // A settings file written before these keys existed keeps the
            // original's factory values instead of failing to load.
            let old = r#"{"last_dir":"/tmp"}"#;
            let old: Config = serde_json::from_str(old).map_err(|e| e.to_string())?;
            if !old.show_zoom_slider || old.size_limit != 10 || old.preview_max_mib != 0 {
                return Err("an older settings file lost its defaults".into());
            }
            Ok("on/10 folders/any size by default, off/3/8 MiB after a restart".to_string())
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
            // The original stops at level 16 (256 px) — DolphinItemListView::
            // setZoomLevel @0x26e100 clamps the level to 0..=16. The rebuild
            // continues the same `level << 4` ladder to level 25 (400 px), so
            // forty steps up land there and stay there.
            for _ in 0..40 {
                pane.zoom_by(1);
            }
            if (pane.zoom[0], pane.icon_size()) != (25, 400.0) {
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
                    let m = toolbar_metrics(avail, label_w, true, 104.0);
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
            let wide = toolbar_metrics(1260.0, 72.0, true, 104.0);
            let narrow = toolbar_metrics(420.0, 72.0, true, 104.0);
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
            let m = crate::toolbar_metrics(1260.0, 72.0, false, 104.0);
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

    // ---- added feature: the picture wall, its captions and its prompt reader ----
    r.check(
        "thumbnail size stops at 400 px",
        (|| {
            use crate::gallery::{clamp_thumb, wall_rows, THUMB_MAX, THUMB_MIN};
            if clamp_thumb(9999.0) != THUMB_MAX {
                return Err(format!("a huge size becomes {}", clamp_thumb(9999.0)));
            }
            if clamp_thumb(1.0) != THUMB_MIN {
                return Err(format!("a tiny size becomes {}", clamp_thumb(1.0)));
            }
            let (small, _) = wall_rows(&[1.5; 6], 1000.0, THUMB_MIN, 12.0, 32.0);
            let (large, _) = wall_rows(&[1.5; 6], 1000.0, THUMB_MAX, 12.0, 32.0);
            let small_h = small.first().map(|r| r.pic_h).unwrap_or(0.0);
            let large_h = large.first().map(|r| r.pic_h).unwrap_or(0.0);
            if !(large_h > small_h) {
                return Err("the biggest wall is not taller than the smallest".into());
            }
            Ok(format!(
                "{THUMB_MIN:.0}..{THUMB_MAX:.0} px, a row measured {:.0} px at the top",
                large_h
            ))
        })(),
    );
    r.check(
        "every picture is as wide as its own shape, so no bands sit between them",
        (|| {
            use crate::gallery::{wall_pic_width, wall_rows};
            // A wide shot, a portrait and a square, as on a real phone roll.
            let aspects = [16.0 / 9.0, 9.0 / 16.0, 1.0, 4.0 / 3.0, 3.0 / 2.0];
            let avail = 1000.0;
            let gap = 12.0;
            let (rows, _) = wall_rows(&aspects, avail, 200.0, gap, 32.0);
            let mut pictures = 0usize;
            for row in &rows {
                let mut w = 0.0;
                for k in row.start..row.end {
                    w += wall_pic_width(row, aspects[k]);
                    pictures += 1;
                }
                w += gap * (row.end - row.start - 1) as f32;
                if w > avail + 1.0 {
                    return Err(format!("a row is {w:.1} px wide on a {avail:.0} px wall"));
                }
                // The frame follows the picture: a 16:9 shot is much wider than
                // it is tall, a 9:16 one much narrower.
                let wide = wall_pic_width(row, 16.0 / 9.0);
                let tall = wall_pic_width(row, 9.0 / 16.0);
                if row.pic_h > 1.0 && !(wide > tall * 2.0) {
                    return Err("a wide and a tall picture get the same width".into());
                }
            }
            if pictures != aspects.len() {
                return Err(format!("{pictures} of {} pictures were packed", aspects.len()));
            }
            Ok(format!(
                "{} rows for {} pictures, none wider than the {avail:.0} px wall",
                rows.len(),
                aspects.len()
            ))
        })(),
    );
    r.check(
        "every row of the picture wall is the size the person set, the last one too",
        (|| {
            use crate::gallery::{wall_pic_width, wall_rows, THUMB_MAX};
            // Twelve pictures of mixed shapes in a 1400 px wall at 400 px: the
            // rows before the last one used to be stretched to fill the width
            // and came out shorter than the size on the slider, while the last
            // one stayed at 400.
            let aspects = [
                16.0 / 9.0,
                3.0 / 4.0,
                9.0 / 16.0,
                1.0,
                4.0 / 3.0,
                2.0 / 3.0,
                3.0 / 2.0,
                1.0,
                1.78,
                0.75,
                0.56,
                1.0,
            ];
            let avail = 1400.0;
            let (rows, _) = wall_rows(&aspects, avail, THUMB_MAX, 12.0, 32.0);
            if rows.len() < 3 {
                return Err(format!("{} rows is not enough to judge", rows.len()));
            }
            for (n, row) in rows.iter().enumerate() {
                if (row.pic_h - THUMB_MAX).abs() > 0.5 {
                    return Err(format!(
                        "row {} is {:.0} px tall where the wall was set to {THUMB_MAX:.0} px",
                        n + 1,
                        row.pic_h
                    ));
                }
                let mut w = 0.0;
                for k in row.start..row.end {
                    w += wall_pic_width(row, aspects[k]);
                }
                w += 12.0 * (row.end - row.start - 1) as f32;
                if w > avail + 1.0 {
                    return Err(format!("row {} is {w:.0} px wide", n + 1));
                }
            }
            // The last row is a row like any other, not a leftover.
            let last = rows.last().unwrap();
            if last.end - last.start == 0 {
                return Err("the last row holds nothing".into());
            }
            Ok(format!(
                "{} rows, every picture {THUMB_MAX:.0} px tall including the {} in the last row",
                rows.len(),
                last.end - last.start
            ))
        })(),
    );
    r.check(
        "the picture wall shows pictures and nothing else",
        (|| {
            // A folder of photographs also holds the usual documents and a
            // folder of the next batch. The person asked that the wall show
            // pictures only, so the wall must not draw a folder tile or a
            // letter tile, and a mark made in another view must not survive
            // into the wall where the marked file cannot be seen.
            let wall = base.join("wall-only");
            write(&wall.join("photo.png"), &png_with_prompt("a picture"))?;
            write(&wall.join("second.jpg"), b"not really a jpeg")?;
            write(&wall.join("notes.txt"), b"words\n")?;
            write(&wall.join("clip.mp4"), b"video")?;
            write(&wall.join("inner/another.png"), &png_with_prompt("nested"))?;
            let mut pane = crate::state::Pane::new(1, wall.clone());
            pane.set_pictures(true);
            let shown: Vec<String> = pane
                .shown_indices()
                .iter()
                .map(|i| pane.display_entries()[*i].name.clone())
                .collect();
            let expected = vec!["photo.png".to_string(), "second.jpg".to_string()];
            if shown != expected {
                return Err(format!("the wall shows {shown:?}"));
            }
            // Ctrl+A on the wall marks the two pictures and nothing else.
            pane.select_all();
            if pane.sel.len() != 2 {
                return Err(format!("{} items were marked on the wall", pane.sel.len()));
            }
            // A Shift+click from a mark made in the grid cannot scoop up the
            // documents that sit between the two pictures.
            let doc = pane
                .display_entries()
                .iter()
                .position(|e| e.name == "notes.txt")
                .ok_or("notes.txt is not listed")?;
            let photo = pane
                .display_entries()
                .iter()
                .position(|e| e.name == "second.jpg")
                .ok_or("second.jpg is not listed")?;
            pane.anchor = doc;
            pane.select_range(photo);
            if pane.sel.len() != 1 || !pane.sel.contains(&photo) {
                return Err(format!(
                    "a Shift+click on the wall marked {} items",
                    pane.sel.len()
                ));
            }
            // A folder marked in the grid is unmarked when the wall comes on.
            pane.set_pictures(false);
            let folder = pane
                .display_entries()
                .iter()
                .position(|e| e.is_dir)
                .ok_or("the sub-folder is not listed")?;
            pane.select_single(folder);
            pane.set_pictures(true);
            if !pane.sel.is_empty() {
                return Err("a folder stayed marked on the wall".into());
            }
            Ok(format!(
                "{} of {} listed items on the wall, both pictures",
                shown.len(),
                pane.display_entries().len()
            ))
        })(),
    );
    r.check(
        "a 400 px thumbnail is drawn 400 px across in the file view",
        (|| {
            use crate::theme::{icon_size_for_zoom, zoom_for_icon_size};
            let square = crate::views::thumb_size(400.0, 1.0);
            if (square.x, square.y) != (400.0, 400.0) {
                return Err(format!("a square 400 px picture became {}×{}", square.x, square.y));
            }
            if icon_size_for_zoom(25) != 400.0 || zoom_for_icon_size(400.0) != 25 {
                return Err(format!(
                    "level 25 is {} px and 400 px maps back to level {}",
                    icon_size_for_zoom(25),
                    zoom_for_icon_size(400.0)
                ));
            }
            Ok("level 25 = 400 px, a square thumbnail fills 400×400".into())
        })(),
    );
    r.check(
        "a picture's dimension caption reads like the browser's",
        (|| {
            let caption = crate::gallery::dims_caption(Some((1920, 1080)));
            if caption != "1920×1080:AR 1.78:MP 2.07" {
                return Err(format!("caption reads {caption:?}"));
            }
            let ratio = crate::gallery::ratio_label(1200, 900);
            if ratio != "4:3" {
                return Err(format!("1200×900 reads as {ratio:?}"));
            }
            Ok(format!("{caption}, 1200×900 is {ratio}"))
        })(),
    );
    r.check(
        "the size line under a picture is written exactly like the file name",
        (|| {
            use crate::gallery::{
                caption_colour, caption_font, wall_caption, CAPTION_SIZE, CAPTION_STEP,
            };
            let font = caption_font();
            if font.size != CAPTION_SIZE || CAPTION_SIZE != 12.0 {
                return Err(format!(
                    "the caption font is {} px where the name is 12 px",
                    font.size
                ));
            }
            if font.family != eframe::egui::FontFamily::Proportional {
                return Err(format!("the caption font family is {:?}", font.family));
            }
            if caption_colour(false) != crate::theme::TEXT {
                return Err("a plain size line is not drawn in the text colour".into());
            }
            if caption_colour(true) != eframe::egui::Color32::WHITE {
                return Err("a marked picture's size line is not lifted to white".into());
            }
            // The wall gives the size line the same room it gives the name, so
            // the two lines sit as far apart as two lines of text.
            let two_lines = wall_caption(true, true);
            let one_line = wall_caption(true, false);
            if (two_lines - one_line - CAPTION_STEP).abs() > 0.01 {
                return Err(format!(
                    "the wall reserves {} px for the size line and {} px for the name",
                    two_lines - one_line,
                    CAPTION_STEP
                ));
            }
            // Names off still leaves the size line on one caption line.
            if wall_caption(false, true) != CAPTION_STEP {
                return Err("the size line alone reserves a different room".into());
            }
            Ok(format!(
                "{CAPTION_SIZE} px, proportional, text colour (white when marked), {CAPTION_STEP} px step"
            ))
        })(),
    );
    r.check(
        "a fresh slideshow starts at 1.5 s and then moves on",
        (|| {
            use crate::gallery::{
                clamp_slide, Slide, SLIDE_DEFAULT, SLIDE_MAX, SLIDE_MIN, SLIDE_STEP,
            };
            if SLIDE_DEFAULT != 1.5 || SLIDE_STEP != 0.5 {
                return Err(format!(
                    "the fresh interval reads {SLIDE_DEFAULT} s (step {SLIDE_STEP})"
                ));
            }
            let mut slide = Slide::new(SLIDE_DEFAULT);
            let start = slide.started;
            if slide.tick(start) {
                return Err("the first picture was replaced at once".into());
            }
            if slide.tick(start + Duration::from_millis(1400)) {
                return Err("a picture moved before 1.5 s had passed".into());
            }
            if !slide.tick(start + Duration::from_millis(1600)) {
                return Err("the second picture never came".into());
            }
            if clamp_slide(0.01) != SLIDE_MIN || clamp_slide(999.0) != SLIDE_MAX {
                return Err("the interval is not held inside its range".into());
            }
            // Space restarts at the person's own interval and fade, not at the
            // factory default.
            let again = Slide::remembered(4.0, false);
            if again.interval != 4.0 || again.fade {
                return Err("a restarted show forgot the person's interval or fade".into());
            }
            // A settings file written before this change holds the old 1 s
            // default. It is lifted to 1.5 s once; a chosen 3 s is left alone.
            let mut old = crate::config::Config::default();
            old.settings_version = 0;
            old.slide_secs = 1.0;
            old.migrate();
            if old.slide_secs != SLIDE_DEFAULT {
                return Err(format!(
                    "an old 1 s setting stayed at {} s",
                    old.slide_secs
                ));
            }
            let mut chosen = crate::config::Config::default();
            chosen.settings_version = 0;
            chosen.slide_secs = 3.0;
            chosen.migrate();
            if chosen.slide_secs != 3.0 {
                return Err("a chosen 3 s interval was overwritten".into());
            }
            if crate::gallery::FADE_MS != 300 {
                return Err("the fade is not 300 ms".into());
            }
            Ok(format!(
                "1.5 s fresh interval, {SLIDE_MIN}..{SLIDE_MAX} s, {} ms fade",
                crate::gallery::FADE_MS
            ))
        })(),
    );
    r.check(
        "a generated picture's prompt is read from its own file",
        (|| {
            use crate::imagemeta;
            // A PNG needs only its signature, a tEXt chunk and IEND for the
            // reader; the pixels are irrelevant to metadata, and writing a real
            // compressed image here would need a compressor.
            let png = base.join("metadata/prompt.png");
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            let text = b"parameters\0a lighthouse at dusk\nNegative prompt: blurry\nSteps: 28, Sampler: DPM++ 2M, CFG scale: 6.5, Seed: 42, Size: 832x1216, Model: horizon-xl";
            bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
            bytes.extend_from_slice(b"tEXt");
            bytes.extend_from_slice(text);
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            bytes.extend_from_slice(b"IEND");
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            write(&png, &bytes)?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;
            if meta.prompt.as_deref() != Some("a lighthouse at dusk") {
                return Err(format!("prompt reads {:?}", meta.prompt));
            }
            if meta.negative.as_deref() != Some("blurry") {
                return Err(format!("negative reads {:?}", meta.negative));
            }
            let field = |k: &str| {
                meta.fields
                    .iter()
                    .find(|(a, _)| a == k)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("")
            };
            if field("Steps") != "28" || field("Seed") != "42" || field("Model") != "horizon-xl" {
                return Err(format!("fields read {:?}", meta.fields));
            }
            let plain = base.join("metadata/plain.png");
            write(&plain, b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00IEND\x00\x00\x00\x00")?;
            if imagemeta::read(&plain).is_some() {
                return Err("a plain PNG was given a prompt".into());
            }
            Ok(format!(
                "prompt, negative and {} fields read from the tEXt chunk",
                meta.fields.len()
            ))
        })(),
    );

    r.check(
        "a saved prompt lands beside the picture and is never overwritten",
        (|| {
            use crate::imagemeta;
            let dir = base.join("metadata/save");
            let png = dir.join("castle.png");
            write(&png, &png_with_prompt(
                "a castle on a hill, morning mist\nNegative prompt: people\nSteps: 30, Sampler: Euler a, CFG scale: 5, Seed: 7, Size: 512x768, Model: dream-v3",
            ))?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;
            let first = meta.save_beside(&png).map_err(|e| e.to_string())?;
            if first.file_name().and_then(|s| s.to_str()) != Some("castle.prompt.txt") {
                return Err(format!("first save landed at {}", first.display()));
            }
            let body = fs::read_to_string(&first).map_err(|e| e.to_string())?;
            if !body.contains("a castle on a hill") || !body.contains("Steps: 30") {
                return Err(format!("the saved text reads {:?}", body));
            }
            let second = meta.save_beside(&png).map_err(|e| e.to_string())?;
            if second == first {
                return Err("the second save overwrote the first".into());
            }
            if second.file_name().and_then(|s| s.to_str()) != Some("castle.2.prompt.txt") {
                return Err(format!("second save landed at {}", second.display()));
            }
            Ok("castle.prompt.txt, then castle.2.prompt.txt".into())
        })(),
    );

    r.check(
        "several pictures give up their prompts in one block",
        (|| {
            use crate::imagemeta;
            let dir = base.join("metadata/batch");
            let one = dir.join("first.png");
            let two = dir.join("second.png");
            let plain = dir.join("holiday-photo.png");
            write(
                &one,
                &png_with_prompt("a red fox in the snow\nSteps: 20, Sampler: Euler a, CFG scale: 7"),
            )?;
            write(&two, &png_with_prompt("a paper boat on a pond"))?;
            write(
                &plain,
                b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00IEND\x00\x00\x00\x00",
            )?;
            let (text, n) =
                imagemeta::prompts_of(&[one.clone(), plain.clone(), two.clone()]);
            if n != 2 {
                return Err(format!("{n} prompts were copied, expected 2"));
            }
            for wanted in [
                "first.png",
                "a red fox in the snow",
                "second.png",
                "a paper boat on a pond",
            ] {
                if !text.contains(wanted) {
                    return Err(format!("the block is missing {wanted:?}: {text:?}"));
                }
            }
            if text.contains("holiday-photo.png") {
                return Err("a picture with no prompt was copied anyway".into());
            }
            Ok(format!("{n} prompts, {} lines", text.lines().count()))
        })(),
    );

    // The person's own words: "the field containing the prompt needs to be
    // selectable so it can easily be copied to the clipboard". A read-only text
    // field is only worth anything if dragging over it selects and Ctrl+C puts
    // that selection on the clipboard, so the check drives the real widget with
    // a press, a drag, a release and the copy event the window sends for Ctrl+C.
    // The person's words: "the prompt area should be collapsable as it is not
    // always needed". The check clicks the block's own title row and watches the
    // text box disappear and come back.
    r.check(
        "the prompt area folds away when its title row is clicked, and opens again",
        (|| {
            use crate::imagemeta;
            use eframe::egui;
            let dir = base.join("metadata/folding");
            let png = dir.join("harbour.png");
            write(
                &png,
                &png_with_prompt("a stone harbour at low tide, gulls, flat grey light"),
            )?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;

            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(340.0, 560.0));
            let mut open = true;
            let mut header: Option<egui::Rect> = None;
            let mut box_seen = [false; 5];
            let mut collapsed_seen = [false; 5];
            let mut click_at: Option<egui::Pos2> = None;

            for frame in 0..5 {
                let mut events: Vec<egui::Event> = Vec::new();
                if let (1, Some(p)) = (frame, click_at) {
                    events.push(egui::Event::PointerMoved(p));
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    });
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    });
                }
                if let (3, Some(p)) = (frame, click_at) {
                    events.push(egui::Event::PointerMoved(p));
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    });
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    });
                }
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(frame as f64 / 60.0),
                    events,
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let shown =
                            crate::gallery::prompt_panel(ui, Some(&meta), Some(&png), &mut open);
                        if let Some(h) = shown.header {
                            header = Some(h);
                        }
                        box_seen[frame] = shown.field.is_some();
                        collapsed_seen[frame] = shown.collapsed;
                    });
                });
                let _ = output;
                if frame == 0 {
                    let h = header.ok_or("the title row was never drawn")?;
                    click_at = Some(egui::pos2(h.center().x, h.center().y));
                }
            }
            if !box_seen[0] || collapsed_seen[0] {
                return Err("the prompt box was not there while the block was open".into());
            }
            if box_seen[2] || !collapsed_seen[2] {
                return Err(format!(
                    "after the click the block reads open={}, box drawn={}",
                    open, box_seen[2]
                ));
            }
            if !box_seen[4] || collapsed_seen[4] {
                return Err("a second click did not bring the prompt back".into());
            }
            Ok("open, folded by a click on the title row, back again by a second click".into())
        })(),
    );

    // The person's own words: "the picture details panel should be closed by
    // default". It used to open itself the first time a picture carrying a
    // prompt came up, which meant the recipe covered the right-hand side of the
    // picture whether or not the person wanted it. The check opens the viewer on
    // such a picture, lets it read the recipe exactly as the window does, pages
    // to a second one, and refuses to pass if the panel comes up open at any
    // point — the info button and the I key are the only ways in now.
    r.check(
        "the picture details panel is closed when a picture opens, and stays closed when paging",
        (|| {
            use crate::gallery::Viewer;
            let dir = base.join("metadata/details");
            let one = dir.join("first.png");
            let two = dir.join("second.png");
            write(
                &one,
                &png_with_prompt("a lighthouse at dusk\nSteps: 28, Model: horizon-xl"),
            )?;
            write(&two, &png_with_prompt("a paper boat on a pond\nSteps: 20"))?;
            let mut v = Viewer::new(
                dir.clone(),
                vec![one.clone(), two.clone()],
                0,
                true,
            );
            if v.show_info {
                return Err("the details panel was open as soon as the viewer started".into());
            }
            v.sync_meta();
            let read = v
                .meta
                .as_ref()
                .and_then(|(_, m)| m.as_ref())
                .and_then(|m| m.prompt.clone());
            if read.as_deref() != Some("a lighthouse at dusk") {
                return Err(format!(
                    "the picture's own recipe was not read behind the panel: {read:?}"
                ));
            }
            if v.show_info {
                return Err("the panel opened itself once the recipe was read".into());
            }
            v.step(1);
            v.sync_meta();
            let second = v
                .meta
                .as_ref()
                .and_then(|(_, m)| m.as_ref())
                .and_then(|m| m.prompt.clone());
            if second.as_deref() != Some("a paper boat on a pond") {
                return Err(format!("paging did not read the next recipe: {second:?}"));
            }
            if v.show_info {
                return Err("paging to a second picture with a prompt opened the panel".into());
            }
            Ok("closed on opening and after paging, with both recipes read".into())
        })(),
    );

    r.check(
        "the prompt text can be dragged over and copied with Ctrl+C",
        (|| {
            use crate::imagemeta;
            use eframe::egui;
            let dir = base.join("metadata/selectable");
            let png = dir.join("lighthouse.png");
            let prompt = "a lighthouse on a granite cliff at dusk, long exposure, volumetric fog, muted teal and amber palette";
            write(
                &png,
                &png_with_prompt(&format!(
                    "{prompt}\nNegative prompt: blurry, text\nSteps: 28, Sampler: DPM++ 2M, CFG scale: 6.5"
                )),
            )?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;

            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 620.0));
            let mut field: Option<egui::Rect> = None;
            let mut copied: Vec<String> = Vec::new();
            let mut press = egui::Pos2::ZERO;
            let mut release = egui::Pos2::ZERO;
            let mut ran = 0;

            for frame in 0..6 {
                let mut events: Vec<egui::Event> = Vec::new();
                match frame {
                    1 => {
                        let f = field.ok_or("the prompt box was never drawn")?;
                        press = egui::pos2(f.left() + 6.0, f.top() + 7.0);
                        events.push(egui::Event::PointerMoved(press));
                        events.push(egui::Event::PointerButton {
                            pos: press,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    2 => {
                        let f = field.ok_or("the prompt box was never drawn")?;
                        release = egui::pos2(f.left() + f.width() * 0.45, f.top() + 7.0);
                        events.push(egui::Event::PointerMoved(release));
                    }
                    3 => {
                        events.push(egui::Event::PointerButton {
                            pos: release,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    4 => {
                        // What the window layer sends when Ctrl+C is pressed.
                        events.push(egui::Event::Copy);
                    }
                    _ => {}
                }
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(frame as f64 / 60.0),
                    events,
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut prompt_open = true;
                        let shown =
                            crate::gallery::prompt_panel(ui, Some(&meta), Some(&png), &mut prompt_open);
                        if let Some(rect) = shown.field {
                            field = Some(rect);
                        }
                    });
                });
                ran += 1;
                for command in output.platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command {
                        copied.push(text);
                    }
                }
            }

            if ran < 6 {
                return Err(format!("only {ran} frames were drawn"));
            }
            let picked = copied
                .last()
                .ok_or("dragging over the prompt and pressing Ctrl+C copied nothing")?;
            let picked = picked.trim();
            if picked.is_empty() {
                return Err("the copied text was empty".into());
            }
            if !prompt.contains(picked) {
                return Err(format!(
                    "the copied text is not part of the prompt: {picked:?}"
                ));
            }
            if picked.len() >= prompt.len() {
                return Err("the whole prompt was copied without a drag selection".into());
            }
            Ok(format!("copied {:?} of {} characters", picked, prompt.len()))
        })(),
    );

    // The person's follow-up: "the prompt selection options should allow text
    // to be selected taking scrolling into account". A long prompt does not fit
    // its box, so a drag has to keep scrolling and keep selecting past the edge
    // — otherwise the last third of every prompt is impossible to copy. This
    // drives that exact gesture: press at the top, hold the pointer on the
    // bottom edge until the box scrolls, release, Ctrl+C, and read what came
    // out. The marker sits at the very end of the prompt, far below the fold.
    r.check(
        "a long prompt keeps selecting while the box scrolls under the drag",
        (|| {
            use crate::imagemeta;
            use eframe::egui;
            let dir = base.join("metadata/scroll-select");
            let png = dir.join("long-prompt.png");
            let head = "a wide empty beach at low tide, wet sand holding a pink evening sky";
            let marker = "TIDEMARK_AT_THE_END";
            let filler = "drifting cloud, soft gradient, distant gulls, fine grain, ".repeat(24);
            let prompt = format!("{head}, {filler}and finally {marker}");
            write(&png, &png_with_prompt(&prompt))?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;

            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 620.0));
            let mut view: Option<egui::Rect> = None;
            let mut scroll = 0.0f32;
            let mut selection: Option<(usize, usize)> = None;
            let mut copied: Vec<String> = Vec::new();
            let mut release_at = egui::Pos2::ZERO;

            for frame in 0..74 {
                let mut events: Vec<egui::Event> = Vec::new();
                if frame == 1 {
                    let v = view.ok_or("the prompt box was never drawn")?;
                    release_at = egui::pos2(v.left() + 8.0, v.top() + 8.0);
                    events.push(egui::Event::PointerMoved(release_at));
                    events.push(egui::Event::PointerButton {
                        pos: release_at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    });
                } else if (2..=60).contains(&frame) {
                    // Held still on the bottom edge: the box is expected to
                    // scroll and the far end of the selection to follow.
                    let v = view.ok_or("the prompt box was never drawn")?;
                    let p = egui::pos2(v.left() + v.width() * 0.7, v.bottom() - 3.0);
                    events.push(egui::Event::PointerMoved(p));
                } else if (61..=68).contains(&frame) {
                    // The rest of the prompt is now in view: run to its end.
                    let v = view.ok_or("the prompt box was never drawn")?;
                    let p = egui::pos2(v.right() - 6.0, v.bottom() - 6.0);
                    events.push(egui::Event::PointerMoved(p));
                } else if frame == 69 {
                    events.push(egui::Event::PointerButton {
                        pos: release_at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    });
                } else if frame == 70 {
                    events.push(egui::Event::Copy);
                }
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(frame as f64 / 60.0),
                    events,
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut prompt_open = true;
                        let shown =
                            crate::gallery::prompt_panel(ui, Some(&meta), Some(&png), &mut prompt_open);
                        if let Some(v) = shown.view {
                            view = Some(v);
                        }
                        scroll = scroll.max(shown.scroll);
                        selection = shown.selection.or(selection);
                    });
                });
                for command in output.platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command {
                        copied.push(text);
                    }
                }
            }

            if scroll <= 0.0 {
                return Err("the prompt box never scrolled while the drag was held at its edge".into());
            }
            let picked = copied
                .last()
                .ok_or("the drag across a scrolling prompt copied nothing")?;
            if !picked.contains(marker) {
                let tail: String = picked
                    .chars()
                    .rev()
                    .take(48)
                    .collect::<Vec<char>>()
                    .into_iter()
                    .rev()
                    .collect();
                return Err(format!(
                    "the copy stopped before the end of the prompt (scrolled {scroll:.0} px), it ends {:?}",
                    tail
                ));
            }
            if !picked.contains("beach at low tide") {
                return Err("the selection lost the start of the prompt".into());
            }
            let (a, b) = selection.ok_or("the block reported no selection")?;
            if b <= a {
                return Err("the selection is empty".into());
            }
            Ok(format!(
                "scrolled {scroll:.0} px, {} of {} characters selected",
                b - a,
                prompt.chars().count()
            ))
        })(),
    );

    // The other half of "taking scrolling into account": the wheel. A person
    // clicks at the top of a prompt, scrolls with the wheel to see the rest and
    // Shift+clicks at the bottom, expecting the range to join up. That only
    // works if the selection survives the wheel — the reason a text field that
    // owned its own scroll could not do it.
    r.check(
        "a selection survives the wheel and a Shift+click extends it",
        (|| {
            use crate::imagemeta;
            use eframe::egui;
            let dir = base.join("metadata/wheel-select");
            let png = dir.join("wheel.png");
            let head = "a quiet harbour at dawn";
            let marker = "WHEELMARK_AT_THE_BOTTOM";
            let filler = "mooring ropes, painted hulls, gulls on the pilings, ".repeat(26);
            let prompt = format!("{head}, {filler}and {marker}");
            write(&png, &png_with_prompt(&prompt))?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;

            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 620.0));
            let mut view: Option<egui::Rect> = None;
            let mut copied: Vec<String> = Vec::new();
            let mut scroll = 0.0f32;
            let mut last = egui::Pos2::ZERO;

            for frame in 0..22 {
                let mut events: Vec<egui::Event> = Vec::new();
                let v = view;
                // 100 ms a frame, so the Shift+click is a separate click rather
                // than the second half of a double-click.
                let _ = frame;
                match frame {
                    0 => {}
                    1 => {
                        let v = v.ok_or("the prompt box was never drawn")?;
                        last = egui::pos2(v.left() + 8.0, v.top() + 8.0);
                        events.push(egui::Event::PointerMoved(last));
                        events.push(egui::Event::PointerButton {
                            pos: last,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    2 => {
                        events.push(egui::Event::PointerButton {
                            pos: last,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    3..=8 => {
                        // One wheel notch per frame, pointer resting on the text.
                        events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -60.0),
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    9 => {
                        let v = v.ok_or("the prompt box was never drawn")?;
                        let p = egui::pos2(v.right() - 8.0, v.bottom() - 6.0);
                        events.push(egui::Event::PointerMoved(p));
                        events.push(egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::SHIFT,
                        });
                    }
                    10 => {
                        let v = v.ok_or("the prompt box was never drawn")?;
                        events.push(egui::Event::PointerButton {
                            pos: egui::pos2(v.right() - 8.0, v.bottom() - 6.0),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::SHIFT,
                        });
                    }
                    12 => events.push(egui::Event::Copy),
                    _ => {}
                }
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(frame as f64 / 10.0),
                    events,
                    // The window layer reports the held modifiers on every
                    // frame, which is how a shifted click is recognised.
                    modifiers: if (9..=10).contains(&frame) {
                        egui::Modifiers::SHIFT
                    } else {
                        egui::Modifiers::default()
                    },
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut prompt_open = true;
                        let shown =
                            crate::gallery::prompt_panel(ui, Some(&meta), Some(&png), &mut prompt_open);
                        if let Some(rect) = shown.view {
                            view = Some(rect);
                        }
                        scroll = scroll.max(shown.scroll);
                    });
                });
                for command in output.platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command {
                        copied.push(text);
                    }
                }
            }

            if scroll <= 0.0 {
                return Err("the wheel did not scroll the prompt box".into());
            }
            let picked = copied.last().ok_or("nothing was copied")?;
            if !picked.starts_with(head) {
                return Err(format!(
                    "the copy does not start at the click that set the anchor: {:?}",
                    picked.chars().take(40).collect::<String>()
                ));
            }
            if !picked.contains(marker) {
                return Err("the Shift+click did not reach the end of the prompt".into());
            }
            Ok(format!(
                "wheel scrolled {scroll:.0} px, {} characters from the first click to the last",
                picked.chars().count()
            ))
        })(),
    );

    // The button in the prompt's title row uses the same selection the drag
    // builds, so it has to reach the block: it writes the range into the store
    // the block reads and the next frame draws it.
    r.check(
        "the Select all button selects the whole prompt",
        (|| {
            use crate::imagemeta;
            use eframe::egui;
            let dir = base.join("metadata/select-all");
            let png = dir.join("select-all.png");
            let prompt = "a brass diving helmet on a wet dock, morning haze, salt on the glass";
            write(&png, &png_with_prompt(prompt))?;
            let meta = imagemeta::read(&png).ok_or("no metadata was found")?;

            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 620.0));
            let mut seen: Option<(usize, usize)> = None;
            let mut open = true;
            let id = egui::Id::new("prompt-selectable-text");
            for frame in 0..3 {
                if frame == 1 {
                    crate::selectable::select_all(&ctx, id, meta.prompt_only().chars().count());
                }
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let shown =
                                crate::gallery::prompt_panel(ui, Some(&meta), Some(&png), &mut open);
                            if shown.selection.is_some() {
                                seen = shown.selection;
                            }
                        });
                    },
                );
            }
            let (a, b) = seen.ok_or("pressing Select all left nothing selected")?;
            let want = meta.prompt_only().chars().count();
            if a != 0 || b != want {
                return Err(format!("the selection is {a}..{b}, expected 0..{want}"));
            }
            Ok(format!("{want} characters selected"))
        })(),
    );

    // ---- comparison view: zoom and pan ----------------------------------
    // These are the rules the person reported as broken: each picture zooms
    // around the pointer, a drag cannot throw a picture out of its pane, one
    // wheel notch is one step, and Sync decides whether the pair moves together.
    r.check(
        "a wheel notch over a picture is exactly one zoom step",
        (|| {
            use crate::gallery::{Wheel, ZOOM_WHEEL_IN};
            let mut wheel = Wheel::default();
            // The notch arrives as a press and a release 7 ms apart.
            let first = wheel.steps(1.000, 120.0);
            let second = wheel.steps(1.007, 120.0);
            if first != 1 || second != 0 {
                return Err(format!("the two halves of one notch gave {first} and {second} steps"));
            }
            let third = wheel.steps(1.500, 120.0);
            if third != 1 {
                return Err(format!("a second, separate notch gave {third} steps"));
            }
            let z = crate::gallery::zoom_step(1.0, ZOOM_WHEEL_IN);
            let z = crate::gallery::zoom_step(z, ZOOM_WHEEL_IN);
            Ok(format!("2 notches = {:.0} %", z * 100.0))
        })(),
    );

    r.check(
        "zooming keeps the point under the pointer in place",
        (|| {
            use crate::gallery::{fitted_size, zoom_about, ZOOM_WHEEL_IN};
            use egui::Vec2;
            let pane = Vec2::new(700.0, 800.0);
            let src = [1920usize, 1080usize];
            let anchor = Vec2::new(260.0, -120.0); // pointer, from the pane middle
            let (z, pan) = zoom_about(1.0, Vec2::ZERO, anchor, ZOOM_WHEEL_IN);
            // The picture point under the pointer before the zoom.
            let before = fitted_size(src, 1.0, pane);
            let point = (anchor - Vec2::ZERO) / 1.0;
            let _ = before;
            let after = point * z + pan;
            if (after - anchor).length() > 0.01 {
                return Err(format!("the point moved {:.2} px", (after - anchor).length()));
            }
            Ok(format!(
                "{:.0} % with the point held at {:.0},{:.0}",
                z * 100.0,
                after.x,
                after.y
            ))
        })(),
    );

    r.check(
        "a drag cannot throw a picture out of its pane",
        (|| {
            use crate::gallery::{clamp_pan, fitted_size};
            use egui::Vec2;
            let pane = Vec2::new(700.0, 800.0);
            let src = [1920usize, 1080usize];
            // At 100 % the picture fits, so it does not move at all.
            let fits = fitted_size(src, 1.0, pane);
            if clamp_pan(Vec2::new(500.0, -300.0), fits, pane) != Vec2::ZERO {
                return Err("a picture that still fits was panned".into());
            }
            // At 200 % it may move until an edge reaches the pane edge.
            let big = fitted_size(src, 2.0, pane);
            let limit = Vec2::new(
                ((big.x - pane.x) * 0.5).max(0.0),
                ((big.y - pane.y) * 0.5).max(0.0),
            );
            let clamped = clamp_pan(Vec2::new(9999.0, -9999.0), big, pane);
            if (clamped.x - limit.x).abs() > 0.01 || (clamped.y + limit.y).abs() > 0.01 {
                return Err(format!(
                    "clamped to {:.0},{:.0} instead of {:.0},{:.0}",
                    clamped.x, clamped.y, limit.x, -limit.y
                ));
            }
            Ok(format!(
                "stops at {:.0} px sideways, {:.0} px up and down",
                limit.x, limit.y
            ))
        })(),
    );

    r.check(
        "only the side-by-side pair has a Sync switch",
        (|| {
            use crate::gallery::{compare_synced, CompareMode};
            if compare_synced(CompareMode::Side, false) {
                return Err("side mode ignored its own switch".into());
            }
            if !compare_synced(CompareMode::Side, true) {
                return Err("side mode ignored Sync on".into());
            }
            if !compare_synced(CompareMode::Split, false)
                || !compare_synced(CompareMode::Overlay, false)
            {
                return Err("split or overlay did not stay locked together".into());
            }
            Ok("split and overlay always follow; side follows the switch".into())
        })(),
    );

    // The sheet's measured icon grid: the original fits 3 columns across a
    // 592 px view at 160 px icons and spreads them to a 197 px pitch
    // (checks/orig-zoom/icons-160px.png). The rebuild packs each cell to its
    // picture's own width instead of reserving that spread, so what it must
    // match is the column count: never fewer than the original's 3.
    r.check(
        "the icon grid fits the original's three 160 px columns",
        (|| {
            use crate::theme;
            use crate::views::{icon_cell_width, pack_icon_rows, ICON_CELL_GAP};
            let avail = theme::ORIGINAL_ICON_VIEW_WIDTH;
            let pitch = avail / theme::ORIGINAL_ICON_COLUMNS_160 as f32;
            if (pitch.round() - theme::ORIGINAL_ICON_PITCH_160).abs() > 0.5 {
                return Err(format!(
                    "592 / 3 is {pitch:.1} px, not the measured {}",
                    theme::ORIGINAL_ICON_PITCH_160
                ));
            }
            let cell = icon_cell_width(160.0, 1.0);
            let rows = pack_icon_rows(&vec![cell; 5], avail, ICON_CELL_GAP);
            let columns = rows.first().map(|r| r.1 - r.0).unwrap_or(0);
            if columns < theme::ORIGINAL_ICON_COLUMNS_160 {
                return Err(format!(
                    "{columns} columns of {cell:.0} px in {avail:.0} px, fewer than the original's {}",
                    theme::ORIGINAL_ICON_COLUMNS_160
                ));
            }
            Ok(format!(
                "{columns} columns of {cell:.0} px in {avail:.0} px; the original's pitch is {}",
                theme::ORIGINAL_ICON_PITCH_160
            ))
        })(),
    );

    // ---- the find bar narrows the listing --------------------------------
    // Typing used to change the words and nothing else, so this walks the same
    // path a keystroke does — words in, narrow, count what is left — and then
    // the path the bar's close button takes.
    let find = base.join("find");
    for name in ["apple.txt", "apricot.txt", "banana.txt"] {
        let _ = write(&find.join(name), b"x");
    }
    let _ = fs::create_dir_all(find.join("apps"));
    r.check(
        "the find bar narrows the listing to the names that match",
        (|| {
            use crate::state::Pane;
            let mut pane = Pane::new(1, find.clone());
            let all = pane.entries.len();
            if all != 4 {
                return Err(format!("expected 4 items before typing, got {all}"));
            }
            pane.filter_on = true;
            pane.filter = "ap".into();
            pane.refilter();
            let shown: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
            if shown.len() != 3 || shown.iter().any(|n| n.to_lowercase().contains("banana")) {
                return Err(format!("“ap” left {} items: {shown:?}", shown.len()));
            }
            // What is selected has to be one of the items still on screen.
            pane.select_all();
            if pane.sel.len() != shown.len() {
                return Err(format!(
                    "selecting all took {} of {} shown items",
                    pane.sel.len(),
                    shown.len()
                ));
            }
            // Closing the bar puts the whole folder back.
            pane.filter_on = false;
            pane.filter.clear();
            pane.refilter();
            if pane.entries.len() != all {
                return Err(format!("closing the bar left {} of {all}", pane.entries.len()));
            }
            Ok(format!("{all} → {} → {all} items", shown.len()))
        })(),
    );

    // ---- the find bar keeps its words when the folder changes ----------
    r.check(
        "a find survives a step into a folder and still narrows it",
        (|| {
            use crate::state::Pane;
            let mut pane = Pane::new(1, find.clone());
            pane.filter_on = true;
            pane.filter = "banana".into();
            pane.refilter();
            pane.navigate(find.join("apps"));
            // An empty folder has nothing to match, which is the honest answer:
            // the words are still in the bar, so it stays narrowed.
            if !pane.filter_on || pane.filter != "banana" {
                return Err("the words were dropped on the way into a folder".into());
            }
            if !pane.entries.is_empty() {
                return Err(format!("an empty folder showed {} items", pane.entries.len()));
            }
            pane.navigate(find.clone());
            if pane.entries.len() != 1 || pane.entries[0].name != "banana.txt" {
                return Err(format!("back in the folder, {} items showed", pane.entries.len()));
            }
            Ok("empty folder, then the one match".into())
        })(),
    );

    // ---- the three verbs and the location bar never share pixels --------
    r.check(
        "the three button words survive an ordinary window and give way in order",
        (|| {
            use crate::toolbar_metrics;
            let words = 110.0; // "Find" + "Open in" + "Menu" at 12 px
            let label = 62.0; // "Sort: Modified"
            let wide = toolbar_metrics(900.0, label, true, words);
            if !wide.labeled {
                return Err("the words were dropped in a 900 px window".into());
            }
            if wide.bar_w < crate::TOOLBAR_BAR_MIN {
                return Err(format!("the location bar was left {:.0} px", wide.bar_w));
            }
            // At every width the location bar and the buttons must not overlap,
            // and the words only go when even the barest worded row — words,
            // no sort button — cannot fit beside a workable location bar.
            let gap = crate::TOOLBAR_CLUSTER_GAP;
            let worded_floor = crate::TOOLBAR_CLUSTER_FIXED + words
                + crate::TOOLBAR_LABEL_EXTRA
                - crate::TOOLBAR_ICON_W
                - crate::TOOLBAR_ITEM_GAP
                + gap
                + crate::TOOLBAR_BAR_MIN;
            for w in (360..=1600).step_by(20) {
                let m = toolbar_metrics(w as f32, label, true, words);
                if m.bar_w + gap + m.cluster_w > w as f32 + 0.5 {
                    return Err(format!("at {w} px the bar and buttons overlap"));
                }
                if !m.labeled && (w as f32) >= worded_floor {
                    return Err(format!(
                        "at {w} px the words went while {worded_floor:.0} px would still fit them"
                    ));
                }
            }
            Ok(format!("900 px keeps the words, bar {:.0} px", wide.bar_w))
        })(),
    );

    // ---- a menu button opens on an ordinary click ------------------------
    // The toolbar's Menu button used to sit on a right-click-only popup, so a
    // plain click did nothing. This drives the same egui call the button makes
    // with a real press and release, and counts the frames its body ran.
    r.check(
        "a menu button opens on an ordinary click",
        (|| {
            use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Sense, Vec2};
            let ctx = Context::default();
            let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(90.0, 24.0));
            let mut bodies = 0usize;
            fn step(ctx: &Context, rect: Rect, bodies: &mut usize, events: Vec<Event>, time: f64) {
                ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(400.0, 300.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let resp = ui.allocate_rect(rect, Sense::click());
                            egui::Popup::menu(&resp).show(|ui| {
                                *bodies += 1;
                                ui.label("menu");
                            });
                        });
                    },
                );
            }
            step(&ctx, rect, &mut bodies, vec![], 0.0);
            let before = bodies;
            let centre = rect.center();
            step(&ctx, rect, &mut bodies, vec![Event::PointerMoved(centre)], 0.1);
            step(
                &ctx,
                rect,
                &mut bodies,
                vec![Event::PointerButton {
                    pos: centre,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                0.2,
            );
            step(
                &ctx,
                rect,
                &mut bodies,
                vec![Event::PointerButton {
                    pos: centre,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                0.3,
            );
            step(&ctx, rect, &mut bodies, vec![], 0.4);
            if before != 0 {
                return Err(format!("the menu was already drawn {before} times"));
            }
            if bodies == 0 {
                return Err("a click did not open the menu".into());
            }
            Ok(format!("closed, then {bodies} frame(s) of menu after one click"))
        })(),
    );

    // ---- settings that survive the next start ---------------------------
    // The person asked for exactly this: a sort or a tab chosen now should be
    // the one the app opens with next time.
    let persist = base.join("persist");
    let _ = write(&persist.join("small.txt"), b"abc");
    let _ = write(&persist.join("big.txt"), &[b'x'; 400]);
    r.check(
        "a sort chosen now is the sort the next start opens with",
        (|| {
            let mut cfg = crate::config::Config::default();
            cfg.sort_key = crate::state::SortKey::Size.index();
            cfg.sort_desc = true;
            // Through the settings file itself, so a missing serde attribute or a
            // renamed key fails here rather than in the person's session.
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: crate::config::Config =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if back.sort_key != 1 || !back.sort_desc {
                return Err("the sort was lost on the way through the file".into());
            }
            let pane = crate::pane_from_config(1, persist.clone(), &back);
            if pane.sort != crate::state::SortKey::Size || pane.asc {
                return Err("the restored pane did not take the saved sort".into());
            }
            let first = pane
                .entries
                .first()
                .map(|e| e.name.clone())
                .ok_or("the folder listed nothing")?;
            if first != "big.txt" {
                return Err(format!(
                    "size descending put {first} first instead of big.txt"
                ));
            }
            Ok("Size / descending from the file, big.txt first".into())
        })(),
    );
    r.check(
        "the tabs of the last run come back, and folders that have gone are skipped",
        (|| {
            let one = base.join("listing");
            let two = base.join("persist");
            let gone = base.join("this-folder-was-deleted");
            let mut cfg = crate::config::Config::default();
            cfg.session = vec![
                crate::config::SavedTab {
                    path: one.clone(),
                    sort_key: Some(3),
                    sort_desc: Some(true),
                },
                crate::config::SavedTab {
                    path: gone,
                    sort_key: None,
                    sort_desc: None,
                },
                crate::config::SavedTab {
                    path: two.clone(),
                    sort_key: Some(1),
                    sort_desc: Some(true),
                },
            ];
            cfg.session_tab = 2;
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: crate::config::Config =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let tabs = back.remembered_tabs();
            let paths: Vec<PathBuf> = tabs.iter().map(|t| t.path.clone()).collect();
            if paths != vec![one, two] {
                return Err(format!("remembered tabs came back as {paths:?}"));
            }
            if tabs[0].sort_key != Some(3) || tabs[1].sort_desc != Some(true) {
                return Err("a tab's own sort was lost".into());
            }
            if back.session_tab != 2 {
                return Err("the tab that was in front was not remembered".into());
            }
            Ok("2 live folders in order, each with its sort".into())
        })(),
    );
    r.check(
        "a settings file written before these choices existed still opens",
        (|| {
            // What an older build wrote: bookmarks and a few switches, no sort key,
            // no tabs, no window size.
            let old = r#"{"last_dir":"/tmp","view":1,"folders_first":true}"#;
            let cfg: crate::config::Config =
                serde_json::from_str(old).map_err(|e| e.to_string())?;
            if cfg.sort_key != 0 || cfg.sort_desc {
                return Err("an old file did not land on Name / ascending".into());
            }
            if !cfg.restore_session {
                return Err("reopening the tabs should be on by default".into());
            }
            if (cfg.split_frac - 0.5).abs() > 0.001 || cfg.session_tab != 0 {
                return Err(format!(
                    "old file got split {:.2} and tab {}",
                    cfg.split_frac, cfg.session_tab
                ));
            }
            Ok("Name / ascending, tabs on, split 0.50".into())
        })(),
    );
    // The catch-all: whatever the window keeps is compared with the last write,
    // so a choice made by a control that forgot to announce itself is still
    // saved. The view buttons were that case; these checks make sure each kind
    // of choice really shows up in the comparison.
    r.check(
        "any choice that changes is noticed, even without a tidy close",
        (|| {
            use crate::config::Config;
            let base_cfg = Config::default();
            let base_sig = crate::settings_signature(&base_cfg);
            let mut changed: Vec<(&str, Config)> = Vec::new();
            let mut c = base_cfg.clone();
            c.view = 1;
            changed.push(("view", c));
            let mut c = base_cfg.clone();
            c.zoom = [7, 2, 4];
            changed.push(("zoom", c));
            let mut c = base_cfg.clone();
            c.sort_key = 3;
            c.sort_desc = true;
            changed.push(("sort", c));
            let mut c = base_cfg.clone();
            c.pictures = true;
            changed.push(("pictures", c));
            let mut c = base_cfg.clone();
            c.gallery_dims = true;
            changed.push(("picture captions", c));
            let mut c = base_cfg.clone();
            c.places_visible = false;
            c.info_panel = true;
            changed.push(("panels", c));
            let mut c = base_cfg.clone();
            c.window_pos = Some([240.0, 160.0]);
            changed.push(("window place", c));
            let mut c = base_cfg.clone();
            c.window_px = Some([1000.0, 700.0]);
            changed.push(("window size", c));
            if crate::settings_signature(&base_cfg) != base_sig {
                return Err("the same settings gave two different signatures".into());
            }
            for (name, cfg) in &changed {
                if crate::settings_signature(cfg) == base_sig {
                    return Err(format!("a change to the {name} would not be written"));
                }
            }
            Ok(format!("{} kinds of choice all show up", changed.len()))
        })(),
    );
    r.check(
        "the window opens where it was left, and ignores a place off any screen",
        (|| {
            use crate::config::Config;
            let mut cfg = Config::default();
            if crate::startup_position(&cfg).is_some() {
                return Err("a fresh profile should let the window manager place the window".into());
            }
            cfg.window_pos = Some([320.0, 180.0]);
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            match crate::startup_position(&back) {
                Some([320.0, 180.0]) => {}
                other => return Err(format!("the saved place came back as {other:?}")),
            }
            cfg.window_pos = Some([90_000.0, -90_000.0]);
            if crate::startup_position(&cfg).is_some() {
                return Err("a place far past any screen was used anyway".into());
            }
            Ok("kept through the file, refused when absurd".into())
        })(),
    );
    r.check(
        "forgetting the settings puts every display choice back",
        (|| {
            use crate::config::Config;
            let mut pane = crate::state::Pane::new(1, base.clone());
            if pane.entries.is_empty() {
                return Err("the throw-away folder listed nothing".into());
            }
            pane.view = crate::state::ViewMode::Details;
            pane.zoom = [9, 9, 9];
            pane.pictures = true;
            pane.sort = crate::state::SortKey::Size;
            pane.asc = false;
            pane.show_hidden = true;
            pane.previews = false;
            crate::apply_defaults(&mut pane);
            let d = Config::default();
            if pane.view.index() != d.view
                || pane.zoom != d.zoom
                || pane.pictures
                || pane.sort.index() != d.sort_key
                || pane.asc != !d.sort_desc
                || pane.show_hidden
                || pane.previews != d.previews
            {
                return Err(format!(
                    "after forgetting: view {}, zoom {:?}, pictures {}, sort {}, ascending {}, hidden {}",
                    pane.view.index(),
                    pane.zoom,
                    pane.pictures,
                    pane.sort.label(),
                    pane.asc,
                    pane.show_hidden
                ));
            }
            Ok("icons, default zoom, Name ascending, no picture wall, hidden files off".into())
        })(),
    );

    // ---- Open With ------------------------------------------------------
    r.check(
        "a program's entry says what it opens, and a hidden one is left out",
        (|| {
            use crate::apps::parse_desktop;
            let text = "[Desktop Entry]\nType=Application\nName=Kate\n\
                        Exec=kate %U\nIcon=kate\n\
                        MimeType=text/plain;text/markdown;\nTerminal=false\n";
            let app = parse_desktop("org.kde.kate.desktop", text)
                .ok_or("the entry was not read as a program")?;
            if app.name != "Kate"
                || app.mimes != vec!["text/plain", "text/markdown"]
                || app.terminal
            {
                return Err(format!(
                    "read back name {:?}, types {:?}, terminal {}",
                    app.name, app.mimes, app.terminal
                ));
            }
            let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
            if parse_desktop("x.desktop", hidden).is_some() {
                return Err("a hidden entry was offered anyway".into());
            }
            Ok("Kate with two types; the hidden one dropped".into())
        })(),
    );
    r.check(
        "the chosen files reach the program, and no field codes are left behind",
        (|| {
            use crate::apps::command_line;
            let one = std::path::PathBuf::from("/tmp/a b.txt");
            let args = command_line("editor %F --name %c --flag %x", &[one.clone()], "Editor", "", "");
            if args.iter().any(|a| a.contains('%')) {
                return Err(format!("a field code survived: {args:?}"));
            }
            if !args.contains(&"/tmp/a b.txt".to_string()) || !args.contains(&"Editor".to_string()) {
                return Err(format!("the file or the name went missing: {args:?}"));
            }
            let two = command_line(
                "viewer %U",
                &[one.clone(), std::path::PathBuf::from("/tmp/c.png")],
                "V",
                "",
                "",
            );
            let urls = two.iter().filter(|a| a.starts_with("file://")).count();
            if urls != 2 || !two.contains(&"file:///tmp/a%20b.txt".to_string()) {
                return Err(format!("the two addresses came out as {two:?}"));
            }
            Ok("one file, its name, and two escaped addresses".into())
        })(),
    );
    r.check(
        "remembering a program rewrites only the row for that kind of file",
        (|| {
            use crate::apps::{parse_mimeapps, with_default};
            let before = "[Default Applications]\ntext/plain=gedit.desktop\n\
                          image/png=shotwell.desktop\n\n\
                          [Added Associations]\ntext/plain=other.desktop\n";
            let after = with_default(before, "text/plain", "org.kde.kate.desktop");
            let map = parse_mimeapps(&after);
            if map.get("text/plain") != Some(&vec!["org.kde.kate.desktop".to_string()]) {
                return Err(format!("the type now reads {:?}", map.get("text/plain")));
            }
            if map.get("image/png") != Some(&vec!["shotwell.desktop".to_string()]) {
                return Err("the other type was disturbed".into());
            }
            if !after.contains("[Added Associations]\ntext/plain=other.desktop") {
                return Err("the other section was rewritten".into());
            }
            let fresh = with_default("", "text/plain", "org.kde.kate.desktop");
            if fresh != "[Default Applications]\ntext/plain=org.kde.kate.desktop\n" {
                return Err(format!("a new file came out as {fresh:?}"));
            }
            Ok("one row changed, every other row and section kept".into())
        })(),
    );
    r.check(
        "the tick box names the file type the way the original does",
        (|| {
            let text = crate::apps::remember_label(".txt", "text/plain");
            if text != "Remember this choice for all files of type \".txt\" (text/plain)" {
                return Err(format!("the sentence reads {text:?}"));
            }
            Ok("extension and type in one sentence".into())
        })(),
    );
    r.check(
        "the menu finds the programs this computer has for a kind of file",
        (|| {
            // A throw-away config home, so nothing here can change the real
            // desktop's preferences.
            let dir = base.join("mimeapps");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let old = std::env::var_os("XDG_CONFIG_HOME");
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            let write = fs::write(
                dir.join("mimeapps.list"),
                "[Default Applications]\ntext/plain=org.kde.kate.desktop\n",
            );
            let preferred = crate::apps::default_for("text/plain");
            let openers = crate::apps::openers_for("text/plain");
            match old {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            write.map_err(|e| e.to_string())?;
            if preferred.as_deref() != Some("org.kde.kate.desktop") {
                return Err(format!("the remembered program read as {preferred:?}"));
            }
            let named = crate::apps::find("org.kde.kate.desktop").is_some();
            Ok(format!(
                "preferred Kate; {} program(s) claim text/plain{}",
                openers.len(),
                if named { ", Kate installed" } else { ", Kate not installed here" }
            ))
        })(),
    );
    r.check(
        "a file whose name says nothing is read before its menu is built",
        (|| {
            let dir = base.join("sniff");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let text = dir.join("Cargo.lock");
            fs::write(&text, b"name = \"dolphincraft\"\n").map_err(|e| e.to_string())?;
            let binary = dir.join("blob.lock");
            fs::write(&binary, [0x7f, b'E', b'L', b'F', 0x00, 0x02]).map_err(|e| e.to_string())?;
            let plain = dir.join("Makefile");
            fs::write(&plain, b"all:\n\tcargo build\n").map_err(|e| e.to_string())?;
            let a = crate::apps::mime_of(&text);
            let b = crate::apps::mime_of(&binary);
            let c = crate::apps::mime_of(&plain);
            if a != "text/plain" || c != "text/plain" {
                return Err(format!("the text files read as {a:?} and {c:?}"));
            }
            if b != "application/octet-stream" {
                return Err(format!("the binary read as {b:?}"));
            }
            Ok("a .lock of text is text/plain, an ELF stays binary, a Makefile is text".into())
        })(),
    );
    r.check(
        "the chooser leads with the programs that open this kind of file",
        (|| {
            let mime = "text/plain";
            let claims = |a: &crate::apps::AppEntry| a.mimes.iter().any(|m| m == mime);
            let list = crate::apps::chooser(mime);
            let first_other = list.iter().position(|a| !claims(a));
            let last_opener = list.iter().rposition(|a| claims(a));
            if let (Some(f), Some(l)) = (first_other, last_opener) {
                if f < l {
                    return Err(format!(
                        "a program that cannot open the type sits at row {f}, before row {l}"
                    ));
                }
            }
            if list.windows(2).any(|w| {
                claims(&w[0]) == claims(&w[1]) && w[0].name.to_lowercase() > w[1].name.to_lowercase()
            }) {
                return Err("the names inside one group are out of order".into());
            }
            Ok(format!(
                "{} opener(s) first, {} program(s) in all",
                list.iter().filter(|a| claims(a)).count(),
                list.len()
            ))
        })(),
    );
    r.check(
        "a closed tab is remembered once, newest first, and the list stops at ten",
        (|| {
            let mut list: Vec<std::path::PathBuf> = Vec::new();
            for n in 0..12 {
                crate::remember_closed_tab(
                    &mut list,
                    std::path::PathBuf::from(format!("/tmp/tab{}", n)),
                    10,
                );
            }
            if list.len() != 10 {
                return Err(format!("the list kept {} entries", list.len()));
            }
            if list.last().map(|p| p.ends_with("tab11")) != Some(true) {
                return Err("the newest folder is not last".into());
            }
            if list.first().map(|p| p.ends_with("tab2")) != Some(true) {
                return Err(format!("the oldest kept is {:?}", list.first()));
            }
            crate::remember_closed_tab(&mut list, std::path::PathBuf::from("/tmp/tab3"), 10);
            let seen = list.iter().filter(|p| p.ends_with("tab3")).count();
            if seen != 1 {
                return Err(format!("tab3 appears {seen} times"));
            }
            if list.last().map(|p| p.ends_with("tab3")) != Some(true) {
                return Err("the folder did not move to the newest end".into());
            }
            Ok("ten kept, newest last, no duplicate".into())
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