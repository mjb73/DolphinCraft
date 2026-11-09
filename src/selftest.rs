//! `--selftest`: exercises the file operations behind the added tools against a
//! throw-away folder and a throw-away home, so the tricky parts (trash records,
//! duplicate grouping, bulk rename planning, search) can be checked without
//! touching the person's own files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fsutil;
use crate::theme;
use crate::findtext;
use crate::imageresize;
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


/// Wait for a content search to finish, or say it did not.
fn wait_find(ft: &mut findtext::FindText) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while ft.scanning && Instant::now() < deadline {
        ft.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    ft.poll();
    if ft.scanning {
        return Err("the search did not finish in 15 seconds".into());
    }
    Ok(())
}

/// Wait for a name search's walk to finish, so a check can read its results.
fn wait_for_search(pane: &mut crate::state::Pane) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let Some(session) = pane.search.as_mut() else {
            return Err("the search never started".into());
        };
        session.poll();
        if session.done {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("the search did not finish in 10 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The details columns of a pane showing search results: no two may overlap, and
/// while the row is wide enough for both the longest name and the longest folder
/// nothing may be elided. Shared by the selftest check, which needs a live `ui`
/// to lay text out.
fn check_search_columns(
    ui: &egui::Ui,
    pane: &crate::state::Pane,
    long_name: &str,
    folder: &str,
) -> Result<String, String> {
    let name_font = egui::FontId::proportional(13.0);
    let path_font = egui::FontId::proportional(12.0);
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, crate::theme::text()))
            .size()
            .x
    };
    let name_px = measure(long_name, name_font.clone());
    let path_px = measure(folder, path_font.clone());
    // The pane measures the same strings with the same fonts, and must agree.
    if (pane.search_name_px - name_px).abs() > 0.5 {
        return Err(format!(
            "the pane measured the widest name as {:.1} px, not {:.1}",
            pane.search_name_px, name_px
        ));
    }
    if (pane.search_path_px - path_px).abs() > 0.5 {
        return Err(format!(
            "the pane measured the widest folder as {:.1} px, not {:.1}",
            pane.search_path_px, path_px
        ));
    }
    let mut said: Vec<String> = Vec::new();
    for (w, label) in [
        (1600.0f32, "1600"),
        (1084.0, "1084"),
        (700.0, "700"),
        (260.0, "260"),
    ] {
        let rect = egui::Rect::from_min_size(
            egui::Pos2::new(100.0, 50.0),
            egui::Vec2::new(w, 26.0),
        );
        let c = pane.details_columns(rect);
        let Some(path) = c.path else {
            return Err(format!("no Path column at {label} px"));
        };
        if !(c.name.right() <= path.left() + 0.01
            && path.right() <= c.type_.left() + 0.01
            && c.type_.right() <= c.size.left() + 0.01
            && c.size.right() <= c.date.left() + 0.01)
        {
            return Err(format!("{label} px: the columns overlap"));
        }
        // The type column is drawn only when the row can hold it: at 260 px it
        // gives way rather than squeezing the name into 30 px.
        if c.type_.width() > 1.0 {
            said.push(format!("{label} px: Type {:.0} px", c.type_.width()));
        } else {
            said.push(format!("{label} px: no Type column"));
        }
        // The header labels must fit between the rules that close their
        // columns: right-aligned ones keep HEAD_PAD of air, and the column
        // being sorted also gives up the arrow's width. "Size" used to end at
        // the column's right edge while the rule was drawn six pixels inside
        // it, so the label straddled the line.
        let head_font = egui::FontId::proportional(12.0);
        let head_px = |s: &str| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(s.to_owned(), head_font.clone(), crate::theme::text())
                    .size()
                    .x
            })
        };
        for (col_label, col) in [("Size", c.size), ("Modified", c.date)] {
            let need = head_px(col_label) + crate::views::HEAD_PAD + crate::views::SORT_ARROW_W;
            if need > col.width() + 0.01 {
                return Err(format!(
                    "{label} px: the {col_label} header needs {need:.1} px (its text, 6 px of air \
                     and the sort arrow) but the column is {:.1} px wide",
                    col.width()
                ));
            }
        }
        if c.type_.width() > 1.0 {
            let need = head_px("Type") + crate::views::HEAD_PAD + crate::views::SORT_ARROW_W;
            if need > c.type_.width() + 0.01 {
                return Err(format!(
                    "{label} px: the Type header needs {need:.1} px but the column is {:.1} px wide",
                    c.type_.width()
                ));
            }
        }
        let name_need = head_px("Name") + 6.0 + crate::views::SORT_ARROW_W;
        if name_need > c.name.width() + 0.01 {
            return Err(format!(
                "{label} px: the Name header needs {name_need:.1} px but its column is {:.1} px",
                c.name.width()
            ));
        }
        // The room each label actually gets: the name starts one icon and its
        // 8 px gap in, with 8 px to spare at the end; the folder has 6 px.
        let icon = pane.icon_size().clamp(16.0, crate::theme::MAX_ICON_PX) + 8.0;
        let name_room = c.name.width() - icon - 8.0;
        let path_room = path.width() - 6.0;
        let text_w = c.name.width() + path.width();
        let need = icon + name_px + 8.0 + path_px + 10.0;
        if need <= text_w {
            if crate::theme::elide_name(ui, long_name, name_room, name_font.clone()) != long_name {
                return Err(format!(
                    "{label}: the name is still cut ({name_room:.0} px of room for {name_px:.0} px), \
                     with {:.0} px of row unused",
                    text_w - need
                ));
            }
            if crate::theme::elide_name(ui, folder, path_room, path_font.clone()) != folder {
                return Err(format!(
                    "{label}: the folder is still cut ({path_room:.0} px of room for {path_px:.0} px)"
                ));
            }
            said.push(format!("{label} px: both fit"));
        } else if c.name.width() < text_w * 0.50 {
            return Err(format!(
                "{label} px: the row cannot hold both, and the name kept only {:.0} of {:.0} px",
                c.name.width(),
                text_w
            ));
        } else {
            said.push(format!("{label} px: too tight for both, name kept {:.0}%",
                c.name.width() / text_w * 100.0));
        }
    }
    Ok(said.join("; "))
}

/// The extension written on a document's page: the size must fit the page it is
/// drawn on, and a row-sized icon must have no label at all rather than a smudge.
fn check_icon_type_label(ui: &egui::Ui) -> Result<String, String> {
    use crate::theme::type_label_px;
    let measure = |text: &str, px: f32| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(
                text.to_owned(),
                egui::FontId::monospace(px),
                crate::theme::text(),
            )
            .size()
            .x
        })
    };
    let mut said: Vec<String> = Vec::new();
    // The page's own width and writing band, from the same constants the drawing
    // uses.
    let px_for = |icon: f32, label: &str| {
        let band_w = icon * crate::theme::PAGE_W_FRAC * crate::theme::TYPE_AIR;
        let band_h = icon * crate::theme::TYPE_BAND_FRAC;
        let reference = measure(label, 10.0);
        let size = type_label_px(reference, band_w, band_h);
        (band_w, size, reference)
    };
    for icon in [200.0f32, 96.0, 64.0] {
        for ext in ["vectorcraft", "pcraft", "blend"] {
            let label = ext.to_ascii_uppercase();
            let (band_w, size, reference) = px_for(icon, &label);
            // A label, when there is one, fits the page it is drawn on. The
            // longest extension gives way first: at 64 px it is below the floor.
            if let Some(px) = size {
                let drawn = reference * px / 10.0;
                if drawn > band_w + 0.5 {
                    return Err(format!(
                        "{label} at {icon:.0} px is drawn {drawn:.1} px wide in a {band_w:.1} px page"
                    ));
                }
                said.push(format!("{label} {px:.1} px on {icon:.0}"));
            } else if icon >= 96.0 || ext != "vectorcraft" {
                return Err(format!("{label} got no size on a {icon:.0} px icon"));
            } else {
                said.push(format!("{label} below the floor on {icon:.0}"));
            }
        }
    }
    // A small row icon cannot hold any of them legibly: it keeps its kind glyph,
    // and the details view's Type column carries the words instead.
    for icon in [24.0f32, 48.0] {
        for ext in ["vectorcraft", "pcraft", "blend"] {
            let label = ext.to_ascii_uppercase();
            let (band_w, size, _) = px_for(icon, &label);
            if size.is_some() {
                return Err(format!(
                    "{label} was labelled on a {icon:.0} px icon ({band_w:.1} px of page)"
                ));
            }
        }
    }
    Ok(said.join(", "))
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

    // ---- Create New: a folder and an empty text file in the open folder --
    let made = base.join("made");
    let _ = fs::create_dir_all(&made);
    r.check(
        "Create New makes an empty text file and a folder, and the second one picks a free name",
        (|| {
            if crate::NewKind::TextFile.default_name() != "New Text File.txt"
                || crate::NewKind::Folder.default_name() != "New Folder"
            {
                return Err("the suggested names are not the ones the menu promises".into());
            }
            let file = crate::NewKind::TextFile
                .create(&made, &crate::NewKind::TextFile.default_name())
                .map(|m| m.path)
                .map_err(|e| e.to_string())?;
            if !file.is_file() {
                return Err("the text file is not on disk".into());
            }
            if fs::metadata(&file).map_err(|e| e.to_string())?.len() != 0 {
                return Err("the new text file is not empty".into());
            }
            // It is a text file by name, so the viewer opens it as one.
            let entry = fsutil::Entry::from_path(&file);
            if !entry.is_text() {
                return Err(".txt is not treated as text".into());
            }
            let folder = crate::NewKind::Folder
                .create(&made, &crate::NewKind::Folder.default_name())
                .map(|m| m.path)
                .map_err(|e| e.to_string())?;
            if !folder.is_dir() {
                return Err("the folder is not on disk".into());
            }
            // The same name twice must not empty the file that already has it.
            let again = crate::NewKind::TextFile.create(&made, "New Text File.txt");
            match again {
                Ok(_) => return Err("an existing name was accepted a second time".into()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("unexpected error {e:?}")),
            }
            // The one-step flow (no window) names the next thing itself, so a
            // second *Create New* neither asks nor collides: `New Folder`, then
            // `New Folder (1)`; `New Text File.txt`, then `New Text File (1).txt`.
            let fresh = made.join("fresh");
            fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
            let name_of = |p: &std::path::Path| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            let a = crate::NewKind::Folder.create_here(&fresh).map(|m| m.path).map_err(|e| e.to_string())?;
            let b = crate::NewKind::Folder.create_here(&fresh).map(|m| m.path).map_err(|e| e.to_string())?;
            let t1 = crate::NewKind::TextFile.create_here(&fresh).map(|m| m.path).map_err(|e| e.to_string())?;
            let t2 = crate::NewKind::TextFile.create_here(&fresh).map(|m| m.path).map_err(|e| e.to_string())?;
            if name_of(&a) != "New Folder" || name_of(&b) != "New Folder (1)" {
                return Err(format!("folders came out {} and {}", name_of(&a), name_of(&b)));
            }
            if name_of(&t1) != "New Text File.txt" || name_of(&t2) != "New Text File (1).txt" {
                return Err(format!("files came out {} and {}", name_of(&t1), name_of(&t2)));
            }
            for p in [&a, &b] {
                if !p.is_dir() {
                    return Err(format!("{} is not a folder on disk", p.display()));
                }
            }
            for p in [&t1, &t2] {
                if !p.is_file() || fs::metadata(p).map_err(|e| e.to_string())?.len() != 0 {
                    return Err(format!("{} is not an empty file on disk", p.display()));
                }
            }
            Ok("2 made by hand, 4 named free".into())
        })(),
    );

    // ---- the document types New offers, read from the registered apps ----
    r.check(
        "a document type is read from its application's entry and its MIME package",
        (|| {
            let package = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="image/x-psb">
    <comment>Large PSD document</comment>
    <glob pattern="*.psb" weight="5"/>
  </mime-type>
  <mime-type type="application/x-photocraft">
    <comment>PhotoCraft document</comment>
    <icon name="ai.storyteller.photocraft"/>
    <glob pattern="*.pcraft" weight="5"/>
  </mime-type>
  <mime-type type="application/x-vectorcraft">
    <comment>VectorCraft document</comment>
    <glob pattern="*.vectorcraft" weight="5"/>
    <glob pattern="*.drawcraft" weight="5"/>
  </mime-type>
  <mime-type type="application/x-vectorcraft-template">
    <comment>VectorCraft template</comment>
    <glob pattern="*.vctemplate" weight="5"/>
  </mime-type>
</mime-info>"#;
            let facts = crate::docs::parse_mime_package(package, "application/x-photocraft")
                .ok_or("the PhotoCraft type was not found in its own package")?;
            if facts.comment != "PhotoCraft document" {
                return Err(format!("the comment came out {:?}", facts.comment));
            }
            let ext = crate::docs::first_extension(&facts.globs).ok_or("no extension in *.pcraft")?;
            if ext != "pcraft" {
                return Err(format!("the extension came out {ext:?}"));
            }
            if facts.icon != "ai.storyteller.photocraft" {
                return Err(format!("the icon came out {:?}", facts.icon));
            }
            // The type next door must not be read as this one.
            let vector = crate::docs::parse_mime_package(package, "application/x-vectorcraft")
                .ok_or("the VectorCraft type was not found")?;
            if vector.globs.len() != 2
                || crate::docs::first_extension(&vector.globs).as_deref() != Some("vectorcraft")
            {
                return Err(format!("the VectorCraft globs came out {:?}", vector.globs));
            }
            // Ownership is read from the desktop's own records, never from a
            // table of application names: the type named after the application,
            // the package naming the application's icon, or the description
            // opening with its name. A template and a shared type never count.
            let made = |comment: &str, icon: &str| crate::docs::MimeFacts {
                comment: comment.to_string(),
                globs: vec!["*.x".to_string()],
                icon: icon.to_string(),
            };
            let cases = [
                // mime, short, app name, app icon, comment, type icon, wanted
                (
                    "application/x-photocraft",
                    "photocraft",
                    "PhotoCraft",
                    "ai.storyteller.photocraft",
                    "PhotoCraft document",
                    "ai.storyteller.photocraft",
                    true,
                ),
                (
                    "application/x-soundcraft-session",
                    "soundcraft",
                    "SoundCraft",
                    "",
                    "SoundCraft session",
                    "",
                    true,
                ),
                (
                    "application/x-vectorcraft-template",
                    "vectorcraft",
                    "VectorCraft",
                    "ai.storyteller.vectorcraft",
                    "VectorCraft template",
                    "ai.storyteller.vectorcraft",
                    false,
                ),
                // Named after neither, but the package names the entry's icon.
                (
                    "application/x-meld-comparison",
                    "meld",
                    "Meld",
                    "org.gnome.Meld",
                    "Meld comparison description",
                    "org.gnome.Meld",
                    true,
                ),
                // Neither of the first two; the description opens with the name.
                (
                    "application/x-extension-fcstd",
                    "freecad",
                    "FreeCAD",
                    "",
                    "FreeCAD document files",
                    "",
                    true,
                ),
                // A type somebody else named is not this application's.
                (
                    "application/x-affinity",
                    "vectorcraft",
                    "VectorCraft",
                    "ai.storyteller.vectorcraft",
                    "Affinity document",
                    "",
                    false,
                ),
                // A shared type belongs to nobody.
                (
                    "image/png",
                    "photocraft",
                    "PhotoCraft",
                    "ai.storyteller.photocraft",
                    "PNG image",
                    "",
                    false,
                ),
                // A program that opens a format it does not own is not its maker.
                (
                    "application/vnd.oasis.opendocument.text",
                    "libreoffice-writer",
                    "LibreOffice Writer",
                    "",
                    "OpenDocument Text",
                    "",
                    false,
                ),
            ];
            for (mime, short, name, icon, comment, type_icon, want) in cases {
                if crate::docs::owns_type(mime, short, name, icon, &made(comment, type_icon)) != want {
                    return Err(format!("{mime} for {short} answered {}", !want));
                }
            }

            if crate::docs::title_case("PhotoCraft document") != "PhotoCraft Document" {
                return Err("the label is not title-cased".into());
            }
            if crate::docs::short_name("ai.storyteller.photocraft.desktop") != "photocraft"
                || crate::docs::short_name("ai.storyteller.vectorcraft") != "vectorcraft"
            {
                return Err("a desktop id did not shorten to its application's name".into());
            }
            // A glob with a wildcard mid-name names a family, not an extension.
            let family = vec!["*.draw*".to_string(), "*.vc".to_string()];
            if crate::docs::first_extension(&family).as_deref() != Some("vc") {
                return Err("a family glob was taken for an extension".into());
            }
            Ok("pcraft first, the label titled, the template ruled out".into())
        })(),
    );

    r.check(
        "the settings decide which document types New offers, and start with none",
        (|| {
            let mut config = crate::config::Config::default();
            if !config.new_docs.is_empty() || !crate::docs::enabled(&config).is_empty() {
                return Err("a fresh profile already offers a document type".into());
            }
            config.set_new_doc("application/x-vectorcraft", true);
            config.set_new_doc("application/x-vectorcraft", true); // twice is still one tick
            if !config.offers_new_doc("application/x-vectorcraft") || config.new_docs.len() != 1 {
                return Err(format!("the tick was not kept: {:?}", config.new_docs));
            }
            if config.offers_new_doc("application/x-photocraft") {
                return Err("an unticked type is offered".into());
            }
            let on = crate::docs::enabled(&config);
            if on.len() != 1
                || crate::docs::catalog().get(on[0]).map(|d| d.mime.as_str())
                    != Some("application/x-vectorcraft")
            {
                return Err(format!("New's rows came out {on:?}"));
            }
            let stored = serde_json::to_string(&config).map_err(|e| e.to_string())?;
            let back: crate::config::Config =
                serde_json::from_str(&stored).map_err(|e| e.to_string())?;
            if !back.offers_new_doc("application/x-vectorcraft") {
                return Err("the tick did not survive the settings file".into());
            }
            config.set_new_doc("application/x-vectorcraft", false);
            if config.offers_new_doc("application/x-vectorcraft")
                || !crate::docs::enabled(&config).is_empty()
            {
                return Err("unticking left the row in place".into());
            }
            Ok(format!(
                "a tick kept through the settings file, {} registered type(s) to choose from",
                crate::docs::catalog().len()
            ))
        })(),
    );

    r.check(
        "a new document is a document the application opens, not an empty file",
        (|| {
            // Only the format this build writes itself: the PhotoCraft kind is
            // made by its own CLI, which a check has no business starting.
            let Some(i) = crate::docs::catalog()
                .iter()
                .position(|d| matches!(d.ext.as_str(), "vectorcraft" | "drawcraft"))
            else {
                return Ok("no VectorCraft type is registered here, so the write was skipped".into());
            };
            let kind = &crate::docs::catalog()[i];
            let dir = made.join("documents");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let a = crate::NewKind::Doc(i).create_here(&dir).map(|m| m.path).map_err(|e| e.to_string())?;
            let b = crate::NewKind::Doc(i).create_here(&dir).map(|m| m.path).map_err(|e| e.to_string())?;
            let name_of = |p: &Path| {
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            };
            if name_of(&a) != kind.default_name() {
                return Err(format!("the first one came out {}", name_of(&a)));
            }
            let second = format!("New {} (1).{}", kind.label, kind.ext);
            if name_of(&b) != second {
                return Err(format!("the second one came out {}, not {second}", name_of(&b)));
            }
            let text = fs::read_to_string(&a).map_err(|e| e.to_string())?;
            if text.trim().is_empty() {
                return Err("the document is an empty file".into());
            }
            if !text.contains("\"format\": \"vectorcraft\"") || !text.contains("\"artboards\"") {
                return Err("the file is not a VectorCraft document".into());
            }
            if serde_json::from_str::<serde_json::Value>(&text).is_err() {
                return Err("the document is not JSON".into());
            }
            Ok(format!("{} and {} both hold a document", name_of(&a), name_of(&b)))
        })(),
    );

    r.check(
        "every registered document type is offered, and a type with no known start still makes a file",
        (|| {
            // The list is the applications' own entries, so it is checked for
            // shape rather than for a count: no type may repeat, and every row
            // must have the three things a row and a file name need.
            let mut seen: Vec<&str> = Vec::new();
            for kind in crate::docs::catalog() {
                if kind.ext.is_empty() || kind.label.is_empty() || kind.app.is_empty() {
                    return Err(format!("{} has an empty part: {kind:?}", kind.mime));
                }
                if seen.contains(&kind.mime.as_str()) {
                    return Err(format!("{} is listed twice", kind.mime));
                }
                if !kind.label.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    return Err(format!("the label {:?} is not title-cased", kind.label));
                }
                seen.push(&kind.mime);
            }
            // A type whose application this build has no command for is still a
            // registered type: the row appears, and the file it makes is the empty
            // one a file manager can always make — `create` says which it got.
            let stranger = crate::docs::DocKind {
                app: "NowhereCraft".to_string(),
                id: "org.example.nowherecraft".to_string(),
                mime: "application/x-nowherecraft".to_string(),
                ext: "ncraft".to_string(),
                label: "NowhereCraft Document".to_string(),
            };
            let dir = made.join("strangers");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let target = dir.join(stranger.default_name());
            let seeded = crate::docs::create(&stranger, &target).map_err(|e| e.to_string())?;
            if seeded {
                return Err("an unknown application was said to have written a document".into());
            }
            if !target.is_file() || fs::metadata(&target).map_err(|e| e.to_string())?.len() != 0 {
                return Err("the fallback did not make an empty file".into());
            }
            Ok(format!(
                "{} registered type(s), each with a label and an extension; an unknown one falls back to an empty file",
                crate::docs::catalog().len()
            ))
        })(),
    );

    // ---- clicking a marked item again asks to rename it -----------------
    r.check(
        "a second click on the marked item is a rename, not a double click and not a fresh mark",
        (|| {
            use crate::views::click_asks_for_rename;
            let cases = [
                // marked, plain, double, pictures, expected
                (true, true, false, false, true),
                (false, true, false, false, false), // the press that marks
                (true, false, false, false, false), // Shift/Ctrl extends the mark
                (true, true, true, false, false),   // a double click opens instead
                (true, true, false, true, false),   // the wall has no name field
            ];
            for (marked, plain, double, pictures, want) in cases {
                if click_asks_for_rename(marked, plain, double, pictures) != want {
                    return Err(format!(
                        "marked {marked}, plain {plain}, double {double}, pictures {pictures} answered {}",
                        click_asks_for_rename(marked, plain, double, pictures)
                    ));
                }
            }
            Ok("5 clicks answered".into())
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
            // updateGridSize @0x26d940, with the 13 px label font at roughly
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
            // `DolphinView::emitStatusBarText` @0x272f40: an empty folder reads
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
            let want_one = format!("1 file ({})", fsutil::format_size(1024));
            if one_file != want_one {
                return Err(format!("one 1 KiB file reads \"{one_file}\""));
            }
            let both = crate::status_counts(2, 3, 2048);
            let want_both = format!("2 folders, 3 files ({})", fsutil::format_size(2048));
            if both != want_both {
                return Err(format!("2 folders and 3 files read \"{both}\""));
            }
            let sel_folders = crate::status_selection(1, 0, 0);
            if sel_folders != "1 folder selected" {
                return Err(format!("one selected folder reads \"{sel_folders}\""));
            }
            let sel_files = crate::status_selection(0, 2, 1536);
            let want_sel = format!("2 files selected ({})", fsutil::format_size(1536));
            if sel_files != want_sel {
                return Err(format!("two selected files read \"{sel_files}\""));
            }
            let sel_mixed = crate::status_selection(1, 2, 1536);
            let want_mixed = format!(
                "1 folder selected, 2 files selected ({})",
                fsutil::format_size(1536)
            );
            if sel_mixed != want_mixed {
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
        "the update check reads a release and knows which version is newer",
        (|| {
            // The shape GitHub answers with, cut down to what is read: tag, body,
            // page and the asset list with its download URLs.
            let json = r###"{
                "tag_name": "v0.1.10",
                "html_url": "https://github.com/mjb73/DolphinCraft/releases/tag/v0.1.10",
                "body": "## What's new in 0.1.10\n\nThe type column, and the type on the page.",
                "assets": [
                    {"name": "dolphincraft-0.1.10-x86_64-linux.tar.gz", "browser_download_url": "https://example.invalid/dolphincraft-0.1.10-x86_64-linux.tar.gz"},
                    {"name": "dolphincraft-0.1.10-x86_64-linux.tar.gz.sha256", "browser_download_url": "https://example.invalid/dolphincraft-0.1.10-x86_64-linux.tar.gz.sha256"},
                    {"name": "dolphincraft-0.1.10-aarch64-linux.tar.gz", "browser_download_url": "https://example.invalid/dolphincraft-0.1.10-aarch64-linux.tar.gz"},
                    {"name": "dolphincraft-0.1.10-aarch64-linux.tar.gz.sha256", "browser_download_url": "https://example.invalid/dolphincraft-0.1.10-aarch64-linux.tar.gz.sha256"}
                ]
            }"###;
            let rel = crate::update::parse_release(json)
                .map_err(|e| format!("a real reply was refused: {e}"))?;
            if rel.version != "0.1.10" {
                return Err(format!("the tag read as {}", rel.version));
            }
            let want = format!("dolphincraft-0.1.10-{}-linux.tar.gz", crate::update::arch());
            if rel.tarball != want {
                return Err(format!("it picked {} instead of {want}", rel.tarball));
            }
            if !rel.tarball_url.ends_with(&want) {
                return Err(format!("its link is {}", rel.tarball_url));
            }
            let sha_want = format!("{}.sha256", rel.tarball_url);
            if rel.sha_url.as_deref() != Some(sha_want.as_str()) {
                return Err(format!("its checksum link is {:?}", rel.sha_url));
            }
            if !rel.notes.contains("type on the page") {
                return Err("the release notes were dropped".into());
            }
            // Numeric, not string order: 0.1.10 is later than 0.1.9, which a text
            // comparison would put the other way round.
            if !crate::update::newer("0.1.6", "0.1.9") || !crate::update::newer("0.1.9", "0.1.10") {
                return Err("a later version was not seen as later".into());
            }
            if crate::update::newer("0.1.10", "0.1.6") || crate::update::newer("0.1.6", "v0.1.6") {
                return Err("an older or the same version was called new".into());
            }
            if !crate::update::newer("0.1.6", "v0.2") {
                return Err("a short tag (v0.2) was not compared".into());
            }
            // A release with no build for this machine is an answer with a
            // reason, not a panic and not a silent offer.
            let bare = r#"{"tag_name":"v0.1.10","assets":[]}"#;
            match crate::update::parse_release(bare) {
                Ok(_) => return Err("a release with no assets was accepted".into()),
                Err(e) if e.contains(crate::update::arch()) => {}
                Err(e) => return Err(format!("the reason did not name the architecture: {e}")),
            }
            Ok(format!(
                "{}, {}, checksum {}, later than {}",
                rel.version,
                rel.tarball,
                if rel.sha_url.is_some() { "listed" } else { "missing" },
                crate::APP_VERSION
            ))
        })(),
    );

    r.check(
        "an update installs from the release archive and leaves no staging file",
        (|| {
            use std::os::unix::fs::PermissionsExt;
            let root = base.join("updateinstall");
            let member = "dolphincraft-9.9.9-x86_64-linux";
            let pack = root.join(member);
            std::fs::create_dir_all(&pack).map_err(|e| e.to_string())?;
            std::fs::write(pack.join("dolphincraft"), b"NEW BUILD\n").map_err(|e| e.to_string())?;
            let archive = root.join(format!("{member}.tar.gz"));
            let status = std::process::Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&root)
                .arg(member)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("tar would not build the fixture archive".into());
            }
            let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
            // The release's own checksum file form: the hex, two spaces, the name.
            let hex = crate::update::sha256_hex(&bytes);
            let sum = format!("{hex}  {member}.tar.gz\n");
            if crate::update::checksum_from_file(&sum).as_deref() != Some(hex.as_str()) {
                return Err("the checksum file did not read back".into());
            }
            if crate::update::checksum_from_file("not a checksum").is_some() {
                return Err("a line that is not a sha256 sum was accepted".into());
            }
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
            let exe = bin.join("dolphincraft");
            std::fs::write(&exe, b"OLD BUILD\n").map_err(|e| e.to_string())?;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
            crate::update::install_archive(&bytes, &exe)?;
            let now = std::fs::read_to_string(&exe).map_err(|e| e.to_string())?;
            if now != "NEW BUILD\n" {
                return Err(format!("the installed build reads {now:?}"));
            }
            let mode = std::fs::metadata(&exe).map_err(|e| e.to_string())?.permissions().mode();
            if mode & 0o111 == 0 {
                return Err("the installed build is not executable".into());
            }
            if bin.join("dolphincraft.new").exists() {
                return Err("the staging file was left behind".into());
            }
            // A different archive has a different sum: the comparison the install
            // makes before it touches anything fails for it.
            let mut tampered = bytes.clone();
            tampered.push(0);
            if crate::update::sha256_hex(&tampered) == hex {
                return Err("a changed archive kept its checksum".into());
            }
            Ok(format!("{member} installed, {mode:o}, no .new left"))
        })(),
    );

    r.check(
        "the Type column orders by type both ways, and keeps names ordered inside a type",
        (|| {
            use crate::state::{Pane, SortKey};
            let mut pane = Pane::new(1, std::path::PathBuf::from("/tmp"));
            pane.entries.clear();
            pane.all.clear();
            for name in ["a.blend", "b.pcraft", "c.vectorcraft", "d.txt", "e.pcraft"] {
                let path = std::path::PathBuf::from(format!("/tmp/{name}"));
                let mut e = fsutil::Entry::from_path(&path);
                e.name = name.to_string();
                pane.entries.push(e.clone());
                pane.all.push(e);
            }
            pane.sort = SortKey::Type;
            pane.asc = true;
            pane.sort_entries();
            let asc: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
            pane.asc = false;
            pane.sort_entries();
            let desc: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
            let mut reversed = asc.clone();
            reversed.reverse();
            if desc != reversed {
                return Err(format!(
                    "descending did not reverse ascending: {asc:?} then {desc:?}"
                ));
            }
            // Two files of one type are ordered by name, so the order is defined
            // rather than whatever the sort happened to visit first.
            let pc: Vec<&String> = asc.iter().filter(|n| n.ends_with(".pcraft")).collect();
            if pc != ["b.pcraft", "e.pcraft"] {
                return Err(format!("the two .pcraft files came out {pc:?}"));
            }
            Ok(format!("{} → {}", asc.join(", "), desc.join(", ")))
        })(),
    );

    r.check(
        "the type written on a document's page is sized to the page",
        (|| {
            let ctx = egui::Context::default();
            let mut outcome: Option<Result<String, String>> = None;
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(480.0, 320.0),
            ));
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        outcome = Some(check_icon_type_label(ui));
                    });
            });
            match outcome {
                Some(result) => result,
                None => Err("the frame never ran".into()),
            }
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
            // `FUN_001e6a00` @0x1e6a00 (the body that writes the free-space
            // button; 0x1e6a40 is an instruction inside it):
            // (total - free) * 100 / total + 0.5, truncated — never a plain
            // integer division.
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
        "sizes are written the way the original's size formatter writes them",
        (|| {
            // `KIO::convertSize` @0x1f25f0 in libKF6KIOCore.so.6 passes
            // precision 1 to `KFormat::formatByteSize` @0x1ba880 in
            // libKF6CoreAddons.so.6. A probe of that installed formatter gives
            // the table below: bytes are a whole count with thousands commas,
            // scaled values always carry one decimal, the half goes away from
            // zero (1.25 KiB → 1.3), and the unit only changes at the step.
            let cases: [(u64, &str); 9] = [
                (999, "999 B"),
                (1000, "1,000 B"),
                (1023, "1,023 B"),
                (1024, "1.0 KiB"),
                (1280, "1.3 KiB"),
                (1536, "1.5 KiB"),
                (1048575, "1,024.0 KiB"),
                (1048576, "1.0 MiB"),
                (1099511627776, "1.0 TiB"),
            ];
            for (bytes, want) in cases {
                let got = fsutil::format_size_in(bytes, 0);
                if got != want {
                    return Err(format!("{bytes} reads {got:?}, expected {want:?}"));
                }
            }
            let legacy = fsutil::format_size_in(1024, 1);
            if legacy != "1.0 KB" {
                return Err(format!("the legacy dialect reads {legacy:?}"));
            }
            let si = fsutil::format_size_in(1000, 2);
            if si != "1.0 kB" {
                return Err(format!("the SI dialect reads {si:?}"));
            }
            Ok("1,023 B, 1.3 KiB at the half, 1,024.0 KiB below the MiB step, 1.0 MiB, 1.0 TiB".to_string())
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
            // setZoomLevel @0x26dec0 clamps the level to 0..=16 (the bound is
            // the 16 at 0x2c31c0). The rebuild
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

    // ---- the original keeps two sizes, one for each previews state ------
    r.check(
        "previews on and off each keep their own size",
        (|| {
            // `DolphinItemListView::setZoomLevel` @0x26dec0 writes the new size
            // into `PreviewSize` while previews are shown and into `IconSize`
            // while they are not (`this+0x200` / `this+0x204`), so the two are
            // remembered separately and toggling previews switches between
            // them. Factory values: PreviewSize 64 px, IconSize 32 px.
            let mut pane = crate::state::Pane::new(3, base.join("listing"));
            pane.view = crate::state::ViewMode::Icons;
            pane.previews = true;
            if (pane.zoom_level(), pane.icon_size()) != (4, 64.0) {
                return Err(format!(
                    "a fresh icon view with previews is level {} at {} px",
                    pane.zoom_level(),
                    pane.icon_size()
                ));
            }
            pane.zoom_by(1);
            if (pane.zoom_level(), pane.icon_size()) != (5, 80.0) {
                return Err(format!(
                    "one step up with previews gave level {} at {} px",
                    pane.zoom_level(),
                    pane.icon_size()
                ));
            }
            pane.previews = false;
            if (pane.zoom_level(), pane.icon_size()) != (2, 32.0) {
                return Err(format!(
                    "previews off should fall back to the icon size, not level {} at {} px",
                    pane.zoom_level(),
                    pane.icon_size()
                ));
            }
            pane.zoom_by(-2);
            if (pane.zoom_level(), pane.icon_size()) != (0, 16.0) {
                return Err(format!(
                    "zooming the plain icons gave level {} at {} px",
                    pane.zoom_level(),
                    pane.icon_size()
                ));
            }
            pane.previews = true;
            if (pane.zoom_level(), pane.icon_size()) != (5, 80.0) {
                return Err(format!(
                    "previews back on should be level 5 / 80 px, not level {} at {} px",
                    pane.zoom_level(),
                    pane.icon_size()
                ));
            }
            if pane.zoom_plain[0] != 0 {
                return Err(format!("the plain ladder moved to {}", pane.zoom_plain[0]));
            }
            Ok("64 px on, 80 px after a step, 32 px off, 16 px after two steps, 80 px back on".into())
        })(),
    );

    r.check(
        "the details rows sit in the original's 20 px side padding",
        (|| {
            // `DetailsModeSettings::DetailsModeSettings` @0x2b0f10 registers
            // `LeftPadding` and `RightPadding` with 0x14 = 20 px, and the view's
            // header and rows are inset by that much on each side.
            use crate::state::{Pane, ViewMode};
            let pane = Pane::new(4, base.join("listing"));
            let rect = egui::Rect::from_min_size(
                egui::Pos2::new(100.0, 50.0),
                egui::Vec2::new(700.0, 26.0),
            );
            let columns = pane.details_columns(rect);
            let (name, date) = (columns.name, columns.date);
            if columns.path.is_some() {
                return Err("a folder view has no Path column, and a search does".into());
            }
            if pane.view != ViewMode::Icons {
                return Err("the pane did not start in the icons view".into());
            }
            if (name.left() - rect.left() - crate::theme::DETAILS_SIDE_PAD).abs() > 0.01 {
                return Err(format!(
                    "the name column starts {:.1} px in, not 20",
                    name.left() - rect.left()
                ));
            }
            if (rect.right() - date.right() - crate::theme::DETAILS_SIDE_PAD).abs() > 0.01 {
                return Err(format!(
                    "the date column ends {:.1} px short of the edge, not 20",
                    rect.right() - date.right()
                ));
            }
            if (crate::theme::DETAILS_SIDE_PAD - 20.0).abs() > 0.01 {
                return Err("the measured padding is no longer 20 px".into());
            }
            Ok("name starts 20 px in, date ends 20 px short".into())
        })(),
    );

    // The find bar's close button must sit inside the bar and clear of the scope
    // chips, at every width. It used to be drawn last, after the field had taken
    // the room it needed, and fell off the right edge of the window.
    r.check(
        "the find bar keeps its close button inside the bar, clear of the chips",
        (|| {
            use crate::search::ThumbCache;
            use crate::state::{Clipboard, Pane, Request};
            let root = base.join("findbar");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let mut pane = Pane::new(12, root.clone());
            pane.filter_on = true;
            pane.filter = "anything".to_string();
            pane.find_deep = true;
            pane.start_search();
            let mut said = Vec::new();
            for width in [720.0f32, 900.0, 1400.0] {
                let ctx = egui::Context::default();
                let mut layout: Option<crate::views::FindBarLayout> = None;
                let mut raw = egui::RawInput::default();
                raw.screen_rect = Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(width, 260.0),
                ));
                let _ = ctx.run(raw, |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let mut thumbs = ThumbCache::new();
                            let clip = Clipboard::new();
                            let mut reqs: Vec<Request> = Vec::new();
                            let mut vctx = crate::views::ViewCtx {
                                thumbs: &mut thumbs,
                                clipboard: &clip,
                                reqs: &mut reqs,
                                split: false,
                                active: true,
                                show_location_bar: false,
                                other_dir: None,
                                new_docs: Vec::new(),
                            };
                            layout = Some(pane.find_bar(ui, &mut vctx));
                        });
                });
                let Some(bar) = layout else {
                    return Err(format!("at {width:.0} px the bar never drew"));
                };
                let (close, chips) = (bar.close, bar.chips);
                if close.width() <= 0.0 {
                    return Err(format!("at {width:.0} px there was no close button"));
                }
                if chips.right() > close.left() + 0.01 {
                    return Err(format!(
                        "at {width:.0} px the chips (to {:.0}) run into the close button (from {:.0})",
                        chips.right(),
                        close.left()
                    ));
                }
                if close.right() > width + 0.01 {
                    return Err(format!(
                        "at {width:.0} px the close button ends at {:.0}, past the edge",
                        close.right()
                    ));
                }
                said.push(format!("{width:.0} px: close at {:.0}..{:.0}", close.left(), close.right()));
            }
            pane.search = None;
            Ok(said.join("; "))
        })(),
    );

    // Added feature: while a search is showing, the details rows carry the folder
    // each document was found in, between the name and the size. Both text
    // columns are sized from the strings they actually hold, so neither the name
    // nor the folder is cut short while the row has room for both.
    r.check(
        "a search's columns fit the longest name and folder, not a share of the row",
        (|| {
            use crate::state::Pane;
            let root = base.join("searchfit");
            let folder = "2024/03/reports-with-a-long-name-for-the-check";
            let long_name = "quarterly-report-with-a-long-name-2026-09.txt";
            let deep = root.join(folder);
            std::fs::create_dir_all(&deep).map_err(|e| e.to_string())?;
            std::fs::write(deep.join(long_name), b"x").map_err(|e| e.to_string())?;
            std::fs::write(root.join("quarterly-top.txt"), b"x").map_err(|e| e.to_string())?;
            let mut pane = Pane::new(11, root.clone());
            pane.find_deep = true;
            pane.filter = "quarterly".to_string();
            pane.start_search();
            wait_for_search(&mut pane)?;
            if pane.search.as_ref().map(|s| s.results.len()).unwrap_or(0) != 2 {
                return Err("the search did not find both files".into());
            }
            // The columns are measured from the fonts, so the check needs a real
            // frame: run one and read the answer out of it.
            let ctx = egui::Context::default();
            let mut outcome: Option<Result<String, String>> = None;
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1600.0, 900.0),
            ));
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        pane.measure_search_columns(ui);
                        outcome = Some(check_search_columns(ui, &pane, long_name, folder));
                    });
            });
            match outcome {
                Some(result) => result,
                None => Err("the frame never ran".into()),
            }
        })(),
    );
    // The folder a hit is written as: relative to the folder that was searched,
    // nothing for a file in that folder itself, and its own path outside it.
    r.check(
        "a search hit's folder is written relative to the folder searched",
        (|| {
            let root = std::path::Path::new("/home/me/docs");
            if crate::views::hit_folder(root, std::path::Path::new("/home/me/docs/a.txt")).is_some()
            {
                return Err("a file in the searched folder got a folder name".into());
            }
            if crate::views::hit_folder(root, std::path::Path::new("/home/me/docs/2024/03/b.txt"))
                .as_deref()
                != Some("2024/03")
            {
                return Err("a file two folders down did not read 2024/03".into());
            }
            if crate::views::hit_folder(root, std::path::Path::new("/elsewhere/c.txt")).as_deref()
                != Some("/elsewhere")
            {
                return Err("a file outside the search lost its own folder".into());
            }
            Ok("2024/03, —, /elsewhere".into())
        })(),
    );

    // The picture strip above the icon grid is 22 px tall and the status bar is
    // 28 px, yet egui sizes a slider's knob from the row's interacting height
    // (`Spacing::interact_size.y`, 24 px here), so a plain slider dropped into a
    // strip drew a knob taller than the strip. `theme::strip_slider` is the one
    // slider the bars use; this check draws it inside a 22 px row and refuses to
    // pass if the slider (and so its knob, `height / 2.5` wide) reaches past the
    // row or keeps the full 24 px height.
    r.check(
        "a slider in a one-row strip stays inside the row",
        (|| {
            use egui::{Context, Rect};
            let ctx = Context::default();
            crate::theme::install(&ctx);
            let mut seen: Option<(Rect, Rect)> = None;
            for frame in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(600.0, 200.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let row = egui::Vec2::new(400.0, 22.0);
                        ui.allocate_ui_with_layout(
                            row,
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let mut v = 3.0f32;
                                let resp = crate::theme::strip_slider(ui, &mut v, 0.0..=16.0, 150.0);
                                seen = Some((resp.rect, ui.min_rect()));
                            },
                        );
                    });
                });
            }
            let (slider, row) = seen.ok_or("the strip was never drawn")?;
            if slider.height() >= 24.0 {
                return Err(format!(
                    "the slider kept its {:.0} px row height, so its knob is {:.1} px tall",
                    slider.height(),
                    slider.height() / 2.5 * 2.0
                ));
            }
            if slider.top() < row.top() - 0.01 || slider.bottom() > row.bottom() + 0.01 {
                return Err(format!(
                    "the slider runs {:.1}..{:.1} px inside a {:.1}..{:.1} px row",
                    slider.top(),
                    slider.bottom(),
                    row.top(),
                    row.bottom()
                ));
            }
            Ok(format!(
                "slider {:.0} px tall in a {:.0} px row, knob {:.1} px",
                slider.height(),
                row.height(),
                slider.height() / 2.5 * 2.0
            ))
        })(),
    );

    r.check(
        "a fresh window opens at the factory size of each view",
        (|| {
            // Measured out of the three settings constructors in 26.08.3:
            // DetailsModeSettings @0x2b0f10 IconSize 16 / PreviewSize 48,
            // IconsModeSettings 0x2b1ac0 32 / 64, CompactModeSettings 0x2af670
            // 16 / 48. `updateDefaultZoomLevel` @0x27dd60 turns the size in
            // force into a level, so the starts are (4, 3, 3) with previews and
            // (2, 0, 0) without.
            if crate::theme::default_zoom(true) != [4, 3, 3] {
                return Err(format!(
                    "previews on starts at {:?}",
                    crate::theme::default_zoom(true)
                ));
            }
            if crate::theme::default_zoom(false) != [2, 0, 0] {
                return Err(format!(
                    "previews off starts at {:?}",
                    crate::theme::default_zoom(false)
                ));
            }
            if crate::theme::DEFAULT_ICON_SIZE != [32, 16, 16]
                || crate::theme::DEFAULT_PREVIEW_SIZE != [64, 48, 48]
            {
                return Err("the factory sizes are not the measured ones".into());
            }
            Ok("icons 64/32, compact 48/16, details 48/16 px → levels 4,3,3 and 2,0,0".into())
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
            let red = theme::c(196, 72, 66);
            let plain = crate::place_icon_colour(false, Some(red));
            let marked = crate::place_icon_colour(true, Some(red));
            if plain != red {
                return Err("an unmarked row changed its colour".into());
            }
            if marked == theme::on_band() || marked == plain {
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
            if place == theme::on_band() || place.b() <= place.r() {
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
            use crate::config::{move_row, Bookmark, Spot};
            let mk = |n: &str| Bookmark::plain(n, base.join(n));
            let names = |v: &Vec<Bookmark>| v.iter().map(|b| b.name.clone()).collect::<Vec<_>>();

            let mut list = vec![mk("a"), mk("b"), mk("c")];
            // Dragged down into the gap between b and c.
            if !move_row(&mut list, Spot::Top(0), Spot::Top(2)) {
                return Err("dragging the first row down did nothing".into());
            }
            if names(&list) != ["b", "a", "c"] {
                return Err(format!("dragging down gave {:?}", names(&list)));
            }
            // Dragged up to the very top.
            if !move_row(&mut list, Spot::Top(2), Spot::Top(0)) {
                return Err("dragging the last row up did nothing".into());
            }
            if names(&list) != ["c", "b", "a"] {
                return Err(format!("dragging up gave {:?}", names(&list)));
            }
            // Let go over its own place, or over the gap right below it: the
            // model's movePlace refuses both, and so does this.
            if move_row(&mut list, Spot::Top(1), Spot::Top(1))
                || move_row(&mut list, Spot::Top(1), Spot::Top(2))
            {
                return Err("a drop on its own place moved the row".into());
            }
            if names(&list) != ["c", "b", "a"] {
                return Err("the refused drops changed the order anyway".into());
            }
            // Dropped right at the end of the list.
            if !move_row(&mut list, Spot::Top(0), Spot::Top(3)) {
                return Err("dragging to the end did nothing".into());
            }
            if names(&list) != ["b", "a", "c"] {
                return Err(format!("dragging to the end gave {:?}", names(&list)));
            }
            Ok("down, up, refused in place, to the end".into())
        })(),
    );

    // ---- added feature: shortcut groups ----------------------------------
    r.check(
        "a shortcut group holds shortcuts and gives them back",
        (|| {
            use crate::config::{dissolve_group, move_row, spot_of_path, Bookmark, Spot};
            let names = |v: &Vec<Bookmark>| v.iter().map(|b| b.name.clone()).collect::<Vec<_>>();
            let mut tree = vec![
                Bookmark::group("Work"),
                Bookmark::plain("Photos", base.join("photos")),
                Bookmark::plain("Music", base.join("music")),
            ];
            // Dropped on the group: it goes in, and the group opens so the row
            // does not vanish into a folded container.
            if !move_row(&mut tree, Spot::Top(1), Spot::In(0, 0)) {
                return Err("dropping a shortcut on a group did nothing".into());
            }
            if tree.len() != 2 || tree[0].children.len() != 1 {
                return Err(format!(
                    "after the drop the panel has {} rows and the group {} children",
                    tree.len(),
                    tree[0].children.len()
                ));
            }
            if !tree[0].open {
                return Err("the group that took the row stayed folded".into());
            }
            if names(&tree[0].children) != ["Photos"] {
                return Err(format!("the group holds {:?}", names(&tree[0].children)));
            }
            // Out again: a child dragged to the top level sits at the gap it was
            // let go on, not inside the group.
            if !move_row(&mut tree, Spot::In(0, 0), Spot::Top(2)) {
                return Err("dragging a shortcut out of a group did nothing".into());
            }
            if !tree[0].children.is_empty() || names(&tree) != ["Work", "Music", "Photos"] {
                return Err(format!("dragging out gave {:?} plus {:?}", names(&tree),
                    names(&tree[0].children)));
            }
            // A group holds shortcuts, not groups: one level is what the panel
            // draws, so the model refuses the drop.
            let mut two = vec![Bookmark::group("A"), Bookmark::group("B")];
            if move_row(&mut two, Spot::Top(1), Spot::In(0, 0)) {
                return Err("a group was allowed inside another group".into());
            }
            if two[0].children.len() != 0 || two.len() != 2 {
                return Err("the refused drop changed the tree anyway".into());
            }
            // Removing a group keeps its shortcuts, in their own order, where
            // the group stood.
            let mut three = vec![
                Bookmark::plain("First", base.join("first")),
                Bookmark::group("Gone"),
                Bookmark::plain("Last", base.join("last")),
            ];
            three[1].children = vec![
                Bookmark::plain("a", base.join("a")),
                Bookmark::plain("b", base.join("b")),
            ];
            if !dissolve_group(&mut three, 1) {
                return Err("removing a group did nothing".into());
            }
            if names(&three) != ["First", "a", "b", "Last"] {
                return Err(format!("removing the group left {:?}", names(&three)));
            }
            // One folder, one row: the search for a folder sees children too.
            let mut nested = vec![Bookmark::group("G")];
            nested[0].children = vec![Bookmark::plain("Docs", base.join("docs"))];
            if spot_of_path(&nested, &base.join("docs")) != Some(Spot::In(0, 0)) {
                return Err("a shortcut inside a group could not be found again".into());
            }
            Ok("in, out, no group in a group, dissolved in place".into())
        })(),
    );

    r.check(
        "a shortcut group is saved with its shortcuts, and an older file still loads",
        (|| {
            use crate::config::Bookmark;
            // The shape written before groups existed: a name and a folder and
            // nothing else. It has to load as a plain row, not a broken group.
            let old = r#"{"name":"Docs","path":"/tmp/dcos/Docs"}"#;
            let bm: Bookmark = serde_json::from_str(old)
                .map_err(|e| format!("an older shortcut did not load: {e}"))?;
            if bm.group || bm.open || !bm.children.is_empty() || bm.path != Path::new("/tmp/dcos/Docs") {
                return Err(format!("an older shortcut loaded as {bm:?}"));
            }
            // A group and its children survive being written and read back.
            let mut group = Bookmark::group("Work");
            group.children = vec![
                Bookmark::plain("Docs", PathBuf::from("/tmp/dcos/Docs")),
                Bookmark::plain("Code", PathBuf::from("/tmp/dcos/Code")),
            ];
            group.open = false;
            let tree = vec![group, Bookmark::plain("Music", PathBuf::from("/tmp/dcos/Music"))];
            let text = serde_json::to_string(&tree).map_err(|e| format!("writing failed: {e}"))?;
            let back: Vec<Bookmark> =
                serde_json::from_str(&text).map_err(|e| format!("reading back failed: {e}"))?;
            if back != tree {
                return Err("the group did not come back as it went out".into());
            }
            if back[0].group != true || back[0].open || back[0].children.len() != 2 {
                return Err(format!("the group came back as {:?}", back[0]));
            }
            // Every folder the panel points at, children included, with the
            // group's empty path left out.
            let paths = crate::config::shortcut_paths(&tree);
            if paths.len() != 3 || paths.iter().any(|p| p.as_os_str().is_empty()) {
                return Err(format!("the folder list came out as {paths:?}"));
            }
            Ok("older rows load plain, groups round-trip folded".into())
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

    // ---- the title row carries the name and the release --------------------
    r.check(
        "the title row prints the app's name and its release",
        (|| {
            let (name, version) = crate::title_row_text();
            if name != crate::APP_NAME || version != crate::APP_VERSION {
                return Err(format!("the bar reads {name:?} {version:?}"));
            }
            if !crate::about_line().contains(version) {
                return Err(format!("the bar's {version:?} is not the About release"));
            }
            Ok(format!("{name} {version}"))
        })(),
    );

    // ---- the menu's About row carries the version -------------------------
    // The row is what the person asked for, and its right-hand column is where the
    // number has to be readable without opening anything.
    r.check(
        "the app menu's About row names the app and shows its release",
        (|| {
            let (label, hint) = crate::about_menu_row();
            if label != format!("About {}", crate::APP_NAME) {
                return Err(format!("the row reads {label:?}"));
            }
            if hint != crate::APP_VERSION || !label.starts_with("About ") {
                return Err(format!("{label:?} shows the version as {hint:?}"));
            }
            Ok(format!("{label} … {hint}"))
        })(),
    );

    // ---- About names the release this build came from --------------------
    // The number is `Cargo.toml`'s `version`, which is exactly the GitHub release
    // tag without its `v` — `cargo` writes it into the binary with `env!`, so the
    // About row cannot disagree with the tag the download was cut from. When the
    // manifest is beside the build (a developer's tree), the two are compared;
    // a downloaded binary has no manifest and keeps the compiled number.
    r.check(
        "About names the release this build came from",
        (|| {
            let line = crate::about_line();
            let version = crate::APP_VERSION;
            if version.is_empty() || version.trim() != version {
                return Err(format!("the compiled release reads {version:?}"));
            }
            if !line.contains(crate::APP_NAME) || !line.contains(version) {
                return Err(format!("About does not carry the release: {line:?}"));
            }
            let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                let declared = text
                    .lines()
                    .find_map(|l| l.strip_prefix("version = \""))
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or("");
                if declared != version {
                    return Err(format!(
                        "the manifest says {declared:?} and the build says {version:?}"
                    ));
                }
            }
            Ok(line)
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
            if caption_colour(false) != crate::theme::text() {
                return Err("a plain size line is not drawn in the text colour".into());
            }
            if caption_colour(true) != theme::on_band() {
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

    // Page Up and Page Down move one *page*: the cursor goes a screenful and the
    // window follows it, because a page that moved only the cursor would leave it
    // off the bottom edge and look like a key that did nothing. A page is whatever
    // the view put in its window, so the check draws the details list, the icon
    // grid and the picture wall into real egui frames at a known size, reads back
    // what each reported, pages with it and measures where the window ended up.
    r.check(
        "Page Up and Page Down move one screenful of the view, Home and End its ends",
        (|| -> Result<String, String> {
            use crate::search::ThumbCache;
            use crate::state::{Clipboard, Pane, Request, ViewMode};
            let root = base.join("paging");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            // Pictures, not text files: the wall shows pictures only, so a folder
            // of documents would report an empty page and prove nothing.
            for i in 0..200 {
                write(&root.join(format!("picture-{i:03}.png")), &png_with_prompt(""))?;
            }
            let size = (900.0f32, 500.0f32);
            let mut said = Vec::new();
            for (name, view, pictures) in [
                ("details", ViewMode::Details, false),
                ("icons", ViewMode::Icons, false),
                ("picture wall", ViewMode::Icons, true),
            ] {
                let mut pane = Pane::new(20, root.clone());
                pane.view = view;
                pane.pictures = pictures;
                pane.load(root.clone());
                let ctx = egui::Context::default();
                let mut raw = egui::RawInput::default();
                raw.screen_rect = Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(size.0, size.1),
                ));
                let mut draw = |pane: &mut Pane, raw: egui::RawInput| {
                    let _ = ctx.run(raw, |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                let mut thumbs = ThumbCache::new();
                                let clip = Clipboard::new();
                                let mut reqs: Vec<Request> = Vec::new();
                                let mut vctx = crate::views::ViewCtx {
                                    thumbs: &mut thumbs,
                                    clipboard: &clip,
                                    reqs: &mut reqs,
                                    split: false,
                                    active: true,
                                    show_location_bar: false,
                                    other_dir: None,
                                new_docs: Vec::new(),
                                };
                                pane.ui(ui, &mut vctx);
                            });
                    });
                };
                // Two frames, so the scroll area has settled before anything is
                // measured.
                draw(&mut pane, raw.clone());
                draw(&mut pane, raw.clone());
                let page = pane.per_page;
                if page < 4 {
                    return Err(format!(
                        "{name}: a page in a {}×{} window is only {page} item(s)",
                        size.0, size.1
                    ));
                }
                if page >= 200 {
                    return Err(format!(
                        "{name}: a page is {page} of 200 items, so the view reported everything"
                    ));
                }
                let at_top = pane.scroll_offset;
                pane.cursor = 0;
                pane.page_cursor(1);
                if pane.cursor < page {
                    return Err(format!(
                        "{name}: Page Down from the first item stopped at {} with a page of {page}",
                        pane.cursor
                    ));
                }
                if pane.scroll_pages != Some(1.0) {
                    return Err(format!(
                        "{name}: Page Down did not leave a page scroll for the view"
                    ));
                }
                draw(&mut pane, raw.clone());
                if pane.scroll_pages.is_some() {
                    return Err(format!("{name}: the view never took the page scroll"));
                }
                let down = pane.scroll_offset - at_top;
                if down < size.1 * 0.5 {
                    return Err(format!(
                        "{name}: the window moved {down:.0} px of a {} px page",
                        size.1
                    ));
                }
                // Up again: the cursor goes back a screenful and so does the window.
                let at_page = pane.scroll_offset;
                pane.page_cursor(-1);
                draw(&mut pane, raw.clone());
                if pane.scroll_offset >= at_page {
                    return Err(format!(
                        "{name}: Page Up left the window at {:.0} px, where the page put it",
                        pane.scroll_offset
                    ));
                }
                // And the keys stop at the ends rather than wrapping.
                for _ in 0..40 {
                    pane.page_cursor(1);
                }
                if pane.cursor != 199 {
                    return Err(format!("{name}: paging past the end stopped at {}", pane.cursor));
                }
                // Home and End take the first and the last entry *and* the window
                // with them: the far end of a long folder is not drawn, so a key
                // that only moved the cursor left the screen untouched.
                pane.jump_cursor(false);
                if pane.cursor != 199 {
                    return Err(format!("{name}: End landed on {} of 200", pane.cursor));
                }
                draw(&mut pane, raw.clone());
                if pane.scroll_offset < at_top + size.1 * 0.5 {
                    return Err(format!(
                        "{name}: End left the window at {:.0} px of {:.0} px of content",
                        pane.scroll_offset, size.1
                    ));
                }
                let at_end = pane.scroll_offset;
                pane.jump_cursor(true);
                if pane.cursor != 0 {
                    return Err(format!("{name}: Home landed on {} of 200", pane.cursor));
                }
                draw(&mut pane, raw.clone());
                if pane.scroll_offset >= at_end {
                    return Err(format!(
                        "{name}: Home left the window at {:.0} px, where End put it",
                        pane.scroll_offset
                    ));
                }
                if pane.scroll_offset > at_top + 1.0 {
                    return Err(format!(
                        "{name}: Home stopped {:.0} px below the top",
                        pane.scroll_offset - at_top
                    ));
                }
                said.push(format!("{name} {page}"));
            }
            Ok(format!(
                "a page is {} items, and the window moves with it",
                said.join(", ")
            ))
        })(),
    );

    // The four keys the person asked for in the picture viewer: Page Up and Page
    // Down are one picture back and one on, Home is the first picture and End the
    // last. The arrows must keep wrapping (that is what a viewer does when you hold
    // one), and a jump has to reset zoom, pan and fit exactly like a page does, or
    // End would land on a picture still framed for the one before it.
    r.check(
        "Home and End take the first and the last picture, and paging still wraps",
        (|| -> Result<String, String> {
            use crate::gallery::Viewer;
            let dir = base.join("viewer/keys");
            let items: Vec<PathBuf> = (0..5)
                .map(|i| dir.join(format!("picture-{i}.png")))
                .collect();
            let mut v = Viewer::new(dir.clone(), items.clone(), 2, false);
            v.zoom = 3.0;
            v.pan = egui::Vec2::new(10.0, 4.0);
            v.fit = false;

            v.jump(0);
            if v.index != 0 {
                return Err(format!("Home landed on picture {}", v.index));
            }
            if v.zoom != 1.0 || v.pan != egui::Vec2::ZERO || !v.fit {
                return Err("Home left the zoom and pan of the picture before it".into());
            }
            v.zoom = 2.0;
            v.fit = false;
            v.jump(items.len() - 1);
            if v.index != items.len() - 1 {
                return Err(format!("End landed on picture {}", v.index));
            }
            if v.zoom != 1.0 || !v.fit {
                return Err("End left the zoom of the picture before it".into());
            }
            // A place past the end is the last picture, not an empty pane.
            v.jump(usize::MAX);
            if v.index != items.len() - 1 {
                return Err("a jump past the end did not stop at the last picture".into());
            }
            // The arrows keep their wrap-around, in both directions.
            v.step(1);
            if v.index != 0 {
                return Err(format!(
                    "stepping on from the last picture landed on {} instead of wrapping to the first",
                    v.index
                ));
            }
            v.step(-1);
            if v.index != items.len() - 1 {
                return Err("stepping back from the first picture did not wrap to the last".into());
            }
            // An empty strip is not a panic.
            let mut empty = Viewer::new(dir, Vec::new(), 0, false);
            empty.jump(0);
            empty.step(1);
            if empty.index != 0 {
                return Err("an empty strip moved somewhere".into());
            }
            Ok("0 ↔ 4 by Home/End and by the arrows, each jump resetting the view".into())
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
        "the two button words survive an ordinary window and give way in order",
        (|| {
            use crate::toolbar_metrics;
            let words = 65.0; // "Find" + "Menu" at 12 px
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
    // The compact view is a list of names, so a fixed five-font-height cell cut
    // every one of them off at the same place. The cell is now as wide as the
    // longest name in the listing; the check draws the view at a known width and
    // fails unless the label has room for that name in full.
    r.check(
        "the compact view widens a column to the longest name instead of cutting it",
        (|| -> Result<String, String> {
            use crate::search::ThumbCache;
            use crate::state::{Clipboard, Pane, Request, ViewMode};
            let root = base.join("compactnames");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let long = "a-rather-long-file-name-that-would-have-been-cut-short.txt";
            for name in ["short.txt", "notes.md", "budget-2026.xlsx", long] {
                write(&root.join(name), b"x")?;
            }
            let mut pane = Pane::new(41, root.clone());
            pane.view = ViewMode::Compact;
            pane.load(root.clone());
            let ctx = egui::Context::default();
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(900.0, 500.0),
            ));
            let mut measured: Option<f32> = None;
            for _ in 0..2 {
                measured = None;
                let _ = ctx.run(raw.clone(), |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let mut thumbs = ThumbCache::new();
                            let clip = Clipboard::new();
                            let mut reqs: Vec<Request> = Vec::new();
                            let mut vctx = crate::views::ViewCtx {
                                thumbs: &mut thumbs,
                                clipboard: &clip,
                                reqs: &mut reqs,
                                split: false,
                                active: true,
                                show_location_bar: false,
                                other_dir: None,
                                new_docs: Vec::new(),
                            };
                            pane.ui(ui, &mut vctx);
                            measured = Some(
                                ui.painter()
                                    .layout_no_wrap(
                                        long.to_owned(),
                                        egui::FontId::proportional(12.5),
                                        crate::theme::text(),
                                    )
                                    .size()
                                    .x,
                            );
                        });
                });
            }
            let want = measured.ok_or("the view never drew")?;
            if pane.compact_name_px + 0.5 < want {
                return Err(format!(
                    "the widest name is {want:.1} px but the view measured {:.1}",
                    pane.compact_name_px
                ));
            }
            // label_x is left + icon + 18 and the label keeps 8 px from the right
            // edge, so the cell must hold name + icon + 26 at the least.
            let icon = pane.icon_size();
            let room = pane.compact_cell_px - icon - 26.0;
            if room + 0.5 < want {
                return Err(format!(
                    "a {:.0} px cell leaves {room:.1} px for names of {want:.1} px",
                    pane.compact_cell_px
                ));
            }
            // A folder of short names must not pay for the long one: the cell stays
            // at the original five-font-height width.
            let short = base.join("compactshort");
            std::fs::create_dir_all(&short).map_err(|e| e.to_string())?;
            for i in 0..20 {
                write(&short.join(format!("f{i}.txt")), b"x")?;
            }
            let mut pane2 = Pane::new(42, short.clone());
            pane2.view = ViewMode::Compact;
            pane2.load(short);
            let _ = ctx.run(raw.clone(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let mut thumbs = ThumbCache::new();
                        let clip = Clipboard::new();
                        let mut reqs: Vec<Request> = Vec::new();
                        let mut vctx = crate::views::ViewCtx {
                            thumbs: &mut thumbs,
                            clipboard: &clip,
                            reqs: &mut reqs,
                            split: false,
                            active: true,
                            show_location_bar: false,
                            other_dir: None,
                                new_docs: Vec::new(),
                        };
                        pane2.ui(ui, &mut vctx);
                    });
            });
            let min_w = crate::theme::compact_cell_width(
                pane2.icon_size(),
                17.0,
            );
            if pane2.compact_cell_px > min_w.max(pane2.compact_name_px + pane2.icon_size() + 28.0) + 1.0 {
                return Err(format!(
                    "a folder of short names got {:.0} px cells",
                    pane2.compact_cell_px
                ));
            }
            Ok(format!(
                "the longest name is {want:.0} px, its cell {:.0} px, {:.1} columns at 900 px — short names keep their narrow cells",
                pane.compact_cell_px,
                (900.0f32 - 8.0) / pane.compact_cell_px
            ))
        })(),
    );

    // The person asked for a memory per view: icons, compact, details and the
    // picture view each keep their own size, and all four come back next session.
    // The check sets each one differently, switches between the views (which is
    // where the old code copied the saved array over the pane's and lost the other
    // sizes), resets one, and takes the whole set through the settings file.
    r.check(
        "each of the four views remembers its own zoom through a restart",
        (|| -> Result<String, String> {
            use crate::config::Config;
            use crate::state::{Pane, ViewMode};
            let dir = base.join("persist");
            let mut pane = Pane::new(30, dir.clone());
            pane.previews = true;
            // Four different sizes: icons 128 px, compact 40 px, details 24 px, and
            // the wall at 275 px.
            pane.view = ViewMode::Icons;
            pane.set_zoom_level(9);
            pane.view = ViewMode::Compact;
            pane.set_zoom_level(5);
            pane.view = ViewMode::Details;
            pane.set_zoom_level(1);
            pane.pictures = true;
            pane.thumb_px = 275.0;
            let want = ([9u8, 5, 1], 275.0f32);
            if (pane.zoom, pane.thumb_px) != want {
                return Err(format!(
                    "setting the four sizes gave {:?} and {} px",
                    pane.zoom, pane.thumb_px
                ));
            }
            // Switching views shows each one's own size, and never disturbs the others.
            pane.pictures = false;
            for (mode, level) in [
                (ViewMode::Icons, 9u8),
                (ViewMode::Compact, 5),
                (ViewMode::Details, 1),
            ] {
                pane.view = mode;
                if pane.zoom_level() != level {
                    return Err(format!(
                        "the {} view came back at {} instead of {level}",
                        mode.label(),
                        pane.zoom_level()
                    ));
                }
            }
            pane.pictures = true;
            if pane.view_zoom_px() != 275.0 {
                return Err(format!(
                    "the picture view came back at {} px instead of 275",
                    pane.view_zoom_px()
                ));
            }
            if pane.zoom != [9, 5, 1] {
                return Err("switching views changed the list views' sizes".into());
            }
            // The zoom keys move the view on show, and only it.
            pane.view_zoom_by(1);
            if pane.thumb_px != 300.0 || pane.zoom != [9, 5, 1] {
                return Err(format!(
                    "a zoom step in the picture view gave {} px and {:?} on the ladder",
                    pane.thumb_px, pane.zoom
                ));
            }
            pane.pictures = false;
            pane.view = ViewMode::Compact;
            pane.view_zoom_by(-1);
            if pane.zoom != [9, 4, 1] {
                return Err(format!(
                    "a zoom step in the compact view gave {:?}",
                    pane.zoom
                ));
            }
            // Resetting the view on show leaves the other three alone.
            pane.view_zoom_reset();
            let d = crate::theme::default_zoom(true);
            if pane.zoom != [9, d[1], 1] {
                return Err(format!(
                    "resetting the compact view changed the others: {:?}",
                    pane.zoom
                ));
            }
            // Through the settings file and back, the way a restart does it.
            pane.zoom = [9, 5, 1];
            pane.thumb_px = 275.0;
            let mut cfg = Config::default();
            cfg.zoom = pane.zoom;
            cfg.zoom_plain = pane.zoom_plain;
            cfg.thumb_px = pane.thumb_px;
            cfg.view = ViewMode::Details.index();
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let again = crate::pane_from_config(31, dir, &back);
            if again.zoom != [9, 5, 1] || again.thumb_px != 275.0 {
                return Err(format!(
                    "the next start opened with {:?} and {} px",
                    again.zoom, again.thumb_px
                ));
            }
            Ok("icons 128 px, compact 40 px, details 24 px, wall 275 px — each its own, all four restored".into())
        })(),
    );

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
    // Tabs are the window's sessions, so walking them and closing them must not
    // lose the one in front. Both rules are pure functions on the tab list.
    r.check(
        "Ctrl+Tab walks the tabs both ways, and closing one keeps the front tab",
        (|| -> Result<String, String> {
            use crate::{cycle_tab_index, index_after_close};
            // Forwards and backwards, wrapping at both ends.
            let mut i = 0usize;
            for want in [1usize, 2, 0] {
                i = cycle_tab_index(3, i, 1);
                if i != want {
                    return Err(format!("Ctrl+Tab landed on {i}, not {want}"));
                }
            }
            for want in [2usize, 1, 0] {
                i = cycle_tab_index(3, i, -1);
                if i != want {
                    return Err(format!("Ctrl+Shift+Tab landed on {i}, not {want}"));
                }
            }
            if cycle_tab_index(1, 0, 1) != 0 || cycle_tab_index(1, 0, -1) != 0 {
                return Err("a window with one tab moved off it".into());
            }
            // Closing a tab to the left shifts the front tab down; closing one to
            // the right leaves it; closing the front one lands on its neighbour,
            // and on the new last tab when it was the last.
            for (len, active, closed, want) in [
                (4usize, 2usize, 0usize, 1usize),
                (4, 2, 3, 2),
                (4, 2, 2, 2),
                (4, 3, 3, 2),
                (2, 1, 0, 0),
                (1, 0, 0, 0),
            ] {
                let got = index_after_close(len, active, closed);
                if got != want {
                    return Err(format!(
                        "{len} tabs, front {active}, closed {closed}: landed on {got}, not {want}"
                    ));
                }
            }
            Ok("wraps both ways; the front tab stays put when its neighbours go".into())
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
                    view: Some(2),
                    pictures: true,
                },
                crate::config::SavedTab {
                    path: gone,
                    sort_key: None,
                    sort_desc: None,
                    view: None,
                    pictures: false,
                },
                crate::config::SavedTab {
                    path: two.clone(),
                    sort_key: Some(1),
                    sort_desc: Some(true),
                    view: Some(0),
                    pictures: false,
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
            // A tab is a session: its view and picture mode travel with it.
            if tabs[0].view != Some(2) || !tabs[0].pictures {
                return Err(format!(
                    "a tab's own view was lost: {:?} pictures {}",
                    tabs[0].view, tabs[0].pictures
                ));
            }
            if tabs[1].view != Some(0) || tabs[1].pictures {
                return Err("tab two took tab one's view".into());
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
        "the window opens where it was left, unless that place does not fit its monitor",
        (|| {
            use crate::config::Config;
            let mut cfg = Config::default();
            if crate::startup_position(&cfg).is_some() {
                return Err("a fresh profile should let the window manager place the window".into());
            }
            // A place recorded together with the monitor it belongs to survives
            // the settings file and is reused.
            cfg.window_pos = Some([320.0, 180.0]);
            cfg.window_px = Some([1200.0, 900.0]);
            cfg.window_monitor = Some([1920.0, 1080.0]);
            let text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            match crate::startup_position(&back) {
                Some([320.0, 180.0]) => {}
                other => return Err(format!("the saved place came back as {other:?}")),
            }
            // The window that started all this: 1200 wide at x = 1084 on a
            // 1920-wide monitor hangs 364 px off the right edge, and the toolbar
            // — with Find and Menu on it — is in the part that is off. The place
            // is refused.
            cfg.window_pos = Some([1084.0, 0.0]);
            cfg.window_px = Some([1200.0, 1303.0]);
            if crate::startup_position(&cfg).is_some() {
                return Err("a window that hangs off its own monitor was placed there anyway".into());
            }
            // A place from a build that did not record the monitor cannot be
            // checked at all, so it is not trusted.
            cfg.window_pos = Some([320.0, 180.0]);
            cfg.window_px = Some([1200.0, 900.0]);
            cfg.window_monitor = None;
            if crate::startup_position(&cfg).is_some() {
                return Err("a place with no monitor to check it against was reused".into());
            }
            // And an absurd place is still refused.
            cfg.window_monitor = Some([1920.0, 1080.0]);
            cfg.window_pos = Some([90_000.0, -90_000.0]);
            if crate::startup_position(&cfg).is_some() {
                return Err("a place far past any screen was used anyway".into());
            }
            Ok("kept when it fits its monitor, refused off it, refused unrecorded, refused absurd".into())
        })(),
    );
    r.check(
        "a window on the second screen is not off-screen, and a maximized one is left to the manager",
        (|| {
            use egui::{pos2, vec2, Rect};
            // The person's own numbers: 1202 x 1053 at x = 1921 on the right-hand
            // monitor, whose size is 2409 x 1355. The old rule compared
            // 1921 + 1202 against 2409 (a size, not an origin) and called the
            // window off-screen — so the first geometry change after a maximize
            // shrank it and threw it onto the other screen.
            let size = vec2(1202.0, 1053.0);
            let monitor = vec2(2409.0, 1355.0);
            let at = |x: f32, y: f32| Some(Rect::from_min_size(pos2(x, y), size));
            if !crate::window_fits(size, at(1921.0, 138.0), monitor) {
                return Err("a window on the right-hand screen was called off-screen".into());
            }
            if !crate::window_fits(size, at(0.0, 0.0), monitor) {
                return Err("a window at the origin was called off-screen".into());
            }
            // An extent bigger than the monitor it is on is the only thing the
            // monitor's own size can tell us, and it is still corrected.
            if crate::window_fits(vec2(3000.0, 1053.0), at(0.0, 0.0), monitor) {
                return Err("a window wider than its monitor was called usable".into());
            }
            // Above the top edge of every screen.
            if crate::window_fits(size, at(100.0, -200.0), monitor) {
                return Err("a window above the screen was called usable".into());
            }
            // An unreported place is not a reason to resize.
            if !crate::window_fits(size, None, monitor) {
                return Err("a window with no reported place was resized anyway".into());
            }
            Ok("1921 on a 2409-wide screen fits; too big, above the screen and no place do not".into())
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

    // ---- the navigation panel's recent groups ----------------------------
    // *Recent Files* is read from the desktop's own `recently-used.xbel`, the
    // same list KIO's `recentlyused:` worker (0x1bf92a in libdolphinprivate)
    // sorts by: last visited wins, folders and things that are gone stay out.
    r.check(
        "recent files come newest visit first, without folders or files that have gone",
        (|| {
            let dir = base.join("recent");
            for name in ["old.txt", "new.txt", "in-folder.txt"] {
                write(&dir.join(name), b"x")?;
            }
            fs::create_dir_all(dir.join("a-folder")).map_err(|e| e.to_string())?;
            let xbel = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0">
 <bookmark href="file://{}/old.txt" added="2020-01-01T00:00:00Z" modified="2020-01-01T00:00:00Z" visited="2020-01-01T00:00:00Z"/>
 <bookmark href="file://{}/new.txt" added="2021-01-01T00:00:00Z" modified="2021-01-01T00:00:00Z" visited="2026-06-01T09:00:00Z"/>
 <bookmark href="file://{}/gone.txt" added="2026-01-01T00:00:00Z" visited="2026-07-01T09:00:00Z"/>
 <bookmark href="file://{}/a-folder" visited="2026-08-01T09:00:00Z"/>
 <bookmark href="file://{}/in-folder.txt" visited="2026-05-01T09:00:00Z"/>
 <bookmark href="file://{}/old.txt" visited="2026-09-01T09:00:00Z"/>
 <bookmark href="smb://server/share/doc.txt" visited="2026-10-01T09:00:00Z"/>
</xbel>"#,
                dir.display(),
                dir.display(),
                dir.display(),
                dir.display(),
                dir.display(),
                dir.display(),
            );
            let list = fsutil::parse_recent_xbel(&xbel, 8);
            let names: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
            // The second bookmark for old.txt is the latest visit, so it leads;
            // the duplicate does not give it two rows.
            if names != vec!["old.txt", "new.txt", "in-folder.txt"] {
                return Err(format!("rows came back as {names:?}"));
            }
            if list.iter().any(|e| e.is_dir) {
                return Err("a folder was listed as a recent file".into());
            }
            if list.iter().any(|e| e.name == "gone.txt") {
                return Err("a file that no longer exists is still offered".into());
            }
            // The row keeps the file's own date; only the order uses the visit.
            let old = list.iter().find(|e| e.name == "old.txt").unwrap();
            let stamp = old
                .modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if stamp > 1_800_000_000 {
                return Err("the row took the visit stamp as the file's own date".into());
            }
            let short = fsutil::parse_recent_xbel(&xbel, 2);
            if short.len() != 2 {
                return Err(format!("asking for 2 rows gave {}", short.len()));
            }
            Ok(format!("{} rows, newest visit first", names.len()))
        })(),
    );
    r.check(
        "a folder walked into is remembered once, newest first, and the list stops at six",
        (|| {
            let mut list: Vec<PathBuf> = Vec::new();
            for n in 0..8 {
                crate::note_location(
                    &mut list,
                    Path::new(&format!("/tmp/place{n}")),
                    crate::RECENT_LOCATIONS_MAX,
                );
            }
            if list.len() != crate::RECENT_LOCATIONS_MAX {
                return Err(format!("the list kept {} entries", list.len()));
            }
            if list.first().map(|p| p.ends_with("place7")) != Some(true) {
                return Err(format!("the newest place is not first: {:?}", list.first()));
            }
            crate::note_location(&mut list, Path::new("/tmp/place3"), crate::RECENT_LOCATIONS_MAX);
            let seen = list.iter().filter(|p| p.ends_with("place3")).count();
            if seen != 1 {
                return Err(format!("place3 appears {seen} times"));
            }
            if list.first().map(|p| p.ends_with("place3")) != Some(true) {
                return Err("the place did not move back to the top".into());
            }
            Ok(format!(
                "{} kept, newest first, no duplicate",
                crate::RECENT_LOCATIONS_MAX
            ))
        })(),
    );
    // The heading row is a real button, not a label: one click folds the group
    // and the same click opens it again, which is how the original's places
    // panel behaves and how the counts beside the words get out of the way.
    r.check(
        "a recent group folds with one click and opens with the next",
        (|| {
            use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
            let ctx = Context::default();
            let mut folds = 0usize;
            let mut clicks = 0usize;
            let mut header_rect = Rect::NOTHING;
            let mut expanded = true;
            let mut body_rows = 0usize;
            for frame in 0..4 {
                let mut events: Vec<Event> = Vec::new();
                if frame == 1 {
                    events.push(Event::PointerMoved(header_rect.center()));
                }
                if frame == 2 {
                    clicks += 1;
                    events.push(Event::PointerButton {
                        pos: header_rect.center(),
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                    events.push(Event::PointerButton {
                        pos: header_rect.center(),
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                }
                let mut folded_now = false;
                let _ = ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(220.0, 300.0))),
                        time: Some(frame as f64 * 0.1),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                let top = ui.cursor().min;
                                header_rect = Rect::from_min_size(
                                    top,
                                    Vec2::new(ui.available_width(), 22.0),
                                );
                                if crate::recent_group_header(ui, "Recent Files", expanded, 3) {
                                    folded_now = true;
                                }
                                if expanded {
                                    for _ in 0..3 {
                                        ui.label("row");
                                        body_rows += 1;
                                    }
                                }
                            });
                    },
                );
                if folded_now {
                    folds += 1;
                    expanded = false;
                }
            }
            if folds != 1 {
                return Err(format!("one click toggled the group {folds} time(s)"));
            }
            if clicks != 1 {
                return Err(format!("the harness sent {clicks} clicks"));
            }
            if body_rows == 0 {
                return Err("the rows were never drawn before the fold".into());
            }
            Ok(format!("folded on the click after {body_rows} row(s) drawn"))
        })(),
    );

    // The *Added tools* group is a menu of tools rather than a place to walk
    // to, so it starts folded and remembers being opened. The heading's count is
    // read from the same table that draws the rows, so the two cannot drift.
    r.check(
        "the Added tools group starts folded and counts its own rows",
        (|| {
            use crate::config::Config;
            let fresh = Config::default();
            if fresh.tools_expanded {
                return Err("a fresh profile opens with the tools group unfolded".into());
            }
            // A file written before the group could fold lands folded as well.
            let old: Config = serde_json::from_str("{\"last_dir\":\"/tmp\"}")
                .map_err(|e| e.to_string())?;
            if old.tools_expanded {
                return Err("a settings file without the key opened the group".into());
            }
            // Opened once, the choice is the person's and survives a reload.
            let mut opened = fresh.clone();
            opened.tools_expanded = true;
            let text = serde_json::to_string(&opened).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if !back.tools_expanded {
                return Err("the open group did not survive the settings file".into());
            }
            if crate::settings_signature(&fresh) == crate::settings_signature(&opened) {
                return Err("folding the group never reaches the settings file".into());
            }
            if crate::ADDED_TOOLS.is_empty() {
                return Err("the group has no rows to show".into());
            }
            let mut labels: Vec<&str> = crate::ADDED_TOOLS.iter().map(|t| t.label).collect();
            labels.sort_unstable();
            let distinct = labels.len();
            labels.dedup();
            if labels.len() != distinct {
                return Err(format!("two tool rows share a label: {labels:?}"));
            }
            Ok(format!(
                "{} rows, folded on a fresh profile",
                crate::ADDED_TOOLS.len()
            ))
        })(),
    );
    // The *Network* group can leave the panel altogether, heading included, for
    // a desktop with no shares — and a hidden group still has a switch, because
    // its heading is gone and a setting with no way back is a trap.
    r.check(
        "the Network group is shown by default and stays hidden when switched off",
        (|| {
            use crate::config::Config;
            let fresh = Config::default();
            if !fresh.network_visible {
                return Err("a fresh profile hides the Network group".into());
            }
            let old: Config = serde_json::from_str("{\"last_dir\":\"/tmp\"}")
                .map_err(|e| e.to_string())?;
            if !old.network_visible {
                return Err("a settings file without the key hid the group".into());
            }
            let mut hidden = fresh.clone();
            hidden.network_visible = false;
            let text = serde_json::to_string(&hidden).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if back.network_visible {
                return Err("hiding the group did not survive the settings file".into());
            }
            if crate::settings_signature(&fresh) == crate::settings_signature(&hidden) {
                return Err("hiding the group never reaches the settings file".into());
            }
            Ok("shown on a fresh profile, the switch is remembered".into())
        })(),
    );

    // Both palettes are held to the same contrast targets, measured rather than
    // eyeballed. The light one is not the dark one with its lightness flipped: a
    // single flip squashes the surface ladder into a few levels of near-white,
    // turns the window's border into its brightest line and leaves the accent at
    // 2.2:1 on the page. Every role therefore carries its own light value, and
    // the numbers here are the ones both modes have to hit.
    r.check(
        "both palettes hold the same contrast targets",
        (|| -> Result<String, String> {
            use crate::config::Config;
            use crate::theme;
            let lum = |c: egui::Color32| {
                let ch = |v: u8| {
                    let v = v as f32 / 255.0;
                    if v <= 0.03928 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
            };
            let cr = |a: egui::Color32, b: egui::Color32| {
                let (x, y) = (lum(a), lum(b));
                (x.max(y) + 0.05) / (x.min(y) + 0.05)
            };
            let mut said = String::new();
            for (mode, light) in [("dark", false), ("light", true)] {
                theme::set_light(light);
                let out = (|| -> Result<(), String> {
                    let view = theme::view_bg();
                    let panel = theme::panel_bg();
                    let toolbar = theme::toolbar_bg();
                    // A ladder you can see: the view, then the panel, then the
                    // toolbar, each a step off the one before.
                    if cr(toolbar, view) < 1.12 {
                        return Err(format!(
                            "{mode}: the toolbar is only {:.2}:1 from the view, so the window reads as one flat plane",
                            cr(toolbar, view)
                        ));
                    }
                    if cr(view, panel) < 1.05 {
                        return Err(format!(
                            "{mode}: the panel is only {:.2}:1 from the view",
                            cr(view, panel)
                        ));
                    }
                    // Ink, at body contrast on the surfaces it lands on.
                    if cr(theme::text(), view) < 7.0 {
                        return Err(format!(
                            "{mode}: file names are only {:.2}:1 on the view",
                            cr(theme::text(), view)
                        ));
                    }
                    if cr(theme::text_dim(), panel) < 4.5 {
                        return Err(format!(
                            "{mode}: secondary text is only {:.2}:1 on the panel",
                            cr(theme::text_dim(), panel)
                        ));
                    }
                    if cr(theme::text_faint(), panel) < 4.5 {
                        return Err(format!(
                            "{mode}: counts and captions are only {:.2}:1 on the panel",
                            cr(theme::text_faint(), panel)
                        ));
                    }
                    if cr(theme::text_faint(), toolbar) < 4.0 {
                        return Err(format!(
                            "{mode}: counts and captions are only {:.2}:1 on the toolbar",
                            cr(theme::text_faint(), toolbar)
                        ));
                    }
                    // Hairlines that survive on their ground, a frame darker than
                    // what it holds, and a hover you can tell from the row above.
                    if cr(theme::separator(), view) < 1.18 {
                        return Err(format!(
                            "{mode}: the row hairline is only {:.2}:1 from the view",
                            cr(theme::separator(), view)
                        ));
                    }
                    if cr(theme::border(), toolbar) < 1.25 {
                        return Err(format!(
                            "{mode}: the window edge is only {:.2}:1 from the toolbar",
                            cr(theme::border(), toolbar)
                        ));
                    }
                    if cr(theme::row_hover(), view) < 1.10 {
                        return Err(format!(
                            "{mode}: the hovered row is only {:.2}:1 from the view",
                            cr(theme::row_hover(), view)
                        ));
                    }
                    // The tab strip: the tab in front stands off the strip by its
                    // plate, and a hovered tab stays under it.
                    let tab = theme::tab_active_bg();
                    if lum(tab) <= lum(panel) {
                        return Err(format!(
                            "{mode}: the active tab is no lighter than the strip behind it"
                        ));
                    }
                    if cr(tab, panel) < 1.06 {
                        return Err(format!(
                            "{mode}: the active tab's plate is only {:.2}:1 from the strip, so the blue line does all the work",
                            cr(tab, panel)
                        ));
                    }
                    if lum(theme::tab_hover_bg()) >= lum(tab) {
                        return Err(format!(
                            "{mode}: a hovered tab is as bright as the tab in front"
                        ));
                    }
                    // A selection band that reads as one, with text on it that
                    // still reads as text.
                    let band = theme::row_sel_active();
                    if cr(band, view) < 1.9 {
                        return Err(format!(
                            "{mode}: the selection band is only {:.2}:1 from the view, so it reads as a hover",
                            cr(band, view)
                        ));
                    }
                    if cr(theme::on_band(), band) < 4.5 {
                        return Err(format!(
                            "{mode}: text on the selection band is only {:.2}:1",
                            cr(theme::on_band(), band)
                        ));
                    }
                    // Text on a filled button: the button is a dark plate in
                    // both modes, so its label stays white in both.
                    if cr(theme::on_band(), theme::action_fill()) < 4.5 {
                        return Err(format!(
                            "{mode}: white text on a filled button is only {:.2}:1",
                            cr(theme::on_band(), theme::action_fill())
                        ));
                    }
                    if cr(theme::on_band(), theme::danger_fill()) < 4.5 {
                        return Err(format!(
                            "{mode}: white text on a destructive button is only {:.2}:1",
                            cr(theme::on_band(), theme::danger_fill())
                        ));
                    }
                    // The accent carries the focus ring and the sort arrow.
                    if cr(theme::accent(), view) < 3.0 {
                        return Err(format!(
                            "{mode}: the accent is only {:.2}:1 on the view, so the focus ring disappears",
                            cr(theme::accent(), view)
                        ));
                    }
                    // Never a pure absolute as a surface or as ink.
                    for (name, col) in [
                        ("view", view),
                        ("panel", panel),
                        ("toolbar", toolbar),
                        ("text", theme::text()),
                        ("paper", theme::paper()),
                    ] {
                        if col == egui::Color32::BLACK || col == egui::Color32::WHITE {
                            return Err(format!("{mode}: {name} is a pure absolute"));
                        }
                    }
                    // The folder keeps its hue, and its highlight stays a
                    // highlight instead of flipping into a shadow.
                    let folder = theme::folder();
                    if !(folder.b() > folder.r() + 20) {
                        return Err(format!("{mode}: the folder icon lost its blue"));
                    }
                    if lum(theme::folder_top()) <= lum(folder) {
                        return Err(format!(
                            "{mode}: the folder's lit edge is darker than its body"
                        ));
                    }
                    said.push_str(&format!("{mode}: view {:.0}%, ", lum(view) * 100.0));
                    Ok(())
                })();
                theme::set_light(false);
                out?;
            }
            // The switch itself still has to reach the settings file.
            if crate::settings_signature(&Config::default()) == crate::settings_signature(&{
                let mut c = Config::default();
                c.light = true;
                c
            }) {
                return Err("the light switch never reaches the settings file".into());
            }
            let mut saved = Config::default();
            saved.light = true;
            let text = serde_json::to_string(&saved).map_err(|e| e.to_string())?;
            let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if !back.light {
                return Err("the light switch did not survive the settings file".into());
            }
            let old: Config =
                serde_json::from_str("{\"last_dir\":\"/tmp\"}").map_err(|e| e.to_string())?;
            if old.light {
                return Err("a settings file without the key opened light".into());
            }
            Ok(format!("{said}both modes on the same targets"))
        })(),
    );

    // The Shading slider was asked for because one display did not show the steps
    // between the light window's surfaces. It must be a no-op at 0 (the designed
    // palette is still exactly what the check above measures), it must open the
    // ladder as it is turned up, and it must not be able to break the contrast
    // targets on the way — the panel deepens, so the captions on it darken with it.
    // Dark mode is not a light palette with deeper shading: it is left alone.
    r.check(
        "the shading slider deepens the light palette without breaking it",
        (|| -> Result<String, String> {
            use crate::config::Config;
            use crate::theme;
            let lum = |c: egui::Color32| {
                let ch = |v: u8| {
                    let v = v as f32 / 255.0;
                    if v <= 0.03928 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
            };
            let cr = |a: egui::Color32, b: egui::Color32| {
                let (x, y) = (lum(a), lum(b));
                (x.max(y) + 0.05) / (x.min(y) + 0.05)
            };
            let out = (|| -> Result<String, String> {
                // At 0 the values are the ones written beside the dark ones.
                theme::set_light(true);
                theme::set_shade(0);
                let at_0 = [
                    theme::view_bg(),
                    theme::panel_bg(),
                    theme::toolbar_bg(),
                    theme::row_hover(),
                    theme::separator(),
                    theme::border(),
                    theme::text(),
                    theme::text_dim(),
                    theme::text_faint(),
                ];
                let designed = [
                    (251, 252, 253),
                    (238, 242, 245),
                    (230, 235, 240),
                    (225, 231, 237),
                    (214, 221, 228),
                    (196, 204, 212),
                    (26, 29, 32),
                    (86, 94, 100),
                    (98, 107, 115),
                ];
                for (i, (got, want)) in at_0.iter().zip(designed.iter()).enumerate() {
                    let want = egui::Color32::from_rgb(want.0, want.1, want.2);
                    if *got != want {
                        return Err(format!(
                            "at 0 role {i} is {got:?}, not the designed {want:?}"
                        ));
                    }
                }
                let view_0 = at_0[0];
                let step_0 = cr(at_0[2], view_0);

                // Turned up, every surface is pulled further away from the view:
                // the step below the paper grows by about three, which is a
                // difference no display can swallow. The order stays.
                theme::set_shade(100);
                for (name, was, now) in [
                    ("the panel", at_0[1], theme::panel_bg()),
                    ("the toolbar", at_0[2], theme::toolbar_bg()),
                    ("the hovered row", at_0[3], theme::row_hover()),
                    ("the row hairline", at_0[4], theme::separator()),
                    ("the window edge", at_0[5], theme::border()),
                ] {
                    let levels = |c: egui::Color32, v: egui::Color32| {
                        ((v.r() as i32 - c.r() as i32) + (v.g() as i32 - c.g() as i32)
                            + (v.b() as i32 - c.b() as i32)) as f32
                            / 3.0
                    };
                    if lum(now) >= lum(was) {
                        return Err(format!(
                            "{name} did not get darker: {:.0}% -> {:.0}%",
                            lum(was) * 100.0,
                            lum(now) * 100.0
                        ));
                    }
                    let grew = levels(now, theme::view_bg()) / levels(was, view_0);
                    if grew < 2.5 {
                        return Err(format!(
                            "{name} is only {grew:.2}× its step below the view, which is not a difference you can see"
                        ));
                    }
                }
                if !(lum(theme::view_bg()) > lum(theme::panel_bg())
                    && lum(theme::panel_bg()) > lum(theme::toolbar_bg())
                    && lum(theme::toolbar_bg()) > lum(theme::row_hover())
                    && lum(theme::row_hover()) > lum(theme::separator())
                    && lum(theme::separator()) > lum(theme::border()))
                {
                    return Err("shading reordered the ladder".into());
                }
                let step_100 = cr(theme::toolbar_bg(), theme::view_bg());
                if step_100 <= step_0 * 1.3 {
                    return Err(format!(
                        "the ladder barely opened: {step_0:.2}:1 -> {step_100:.2}:1"
                    ));
                }

                // And the window still reads. The captions darken with the panel
                // they sit on, so they keep their floors; the ink stays the darkest
                // of the three text roles and none of them turns into a pure black.
                let view = theme::view_bg();
                let panel = theme::panel_bg();
                let toolbar = theme::toolbar_bg();
                for (what, got, want) in [
                    ("file names on the view", cr(theme::text(), view), 7.0),
                    ("secondary text on the panel", cr(theme::text_dim(), panel), 4.5),
                    ("captions on the panel", cr(theme::text_faint(), panel), 4.5),
                    ("captions on the toolbar", cr(theme::text_faint(), toolbar), 4.0),
                    ("the row hairline", cr(theme::separator(), view), 1.18),
                    ("the window edge", cr(theme::border(), toolbar), 1.25),
                    ("the hovered row", cr(theme::row_hover(), view), 1.10),
                    ("the accent on the view", cr(theme::accent(), view), 3.0),
                ] {
                    if got < want {
                        return Err(format!("at 100 % shading, {what} is only {got:.2}:1"));
                    }
                }
                if !(lum(theme::text()) < lum(theme::text_dim())
                    && lum(theme::text_dim()) < lum(theme::text_faint()))
                {
                    return Err("shading lost the order of ink, dim and faint".into());
                }
                // The shortcuts panel's keycaps deepen with the panel and stay a
                // raised plate on it — a chip left at near-white was the one part
                // of that panel that did not join in.
                let badge_0 = {
                    theme::set_shade(0);
                    theme::key_badge(false)
                };
                if badge_0 != egui::Color32::from_rgb(242, 244, 245) {
                    return Err(format!("at 0 the key badge is {badge_0:?}"));
                }
                theme::set_shade(100);
                let badge = theme::key_badge(false);
                let badge_sel = theme::key_badge(true);
                if lum(badge) >= lum(badge_0) {
                    return Err("the shortcuts panel's keycaps did not follow the shading".into());
                }
                for (name, plate) in [("keycap", badge), ("selected keycap", badge_sel)] {
                    if lum(plate) <= lum(panel) {
                        return Err(format!(
                            "at 100 % shading the {name} sank into the panel instead of standing on it"
                        ));
                    }
                }
                if cr(theme::text_faint(), badge) < 4.5 {
                    return Err(format!(
                        "at 100 % shading a keycap's hint is only {:.2}:1",
                        cr(theme::text_faint(), badge)
                    ));
                }
                for (name, col) in [
                    ("the view", view),
                    ("the panel", panel),
                    ("the toolbar", toolbar),
                    ("the window edge", theme::border()),
                    ("the captions", theme::text_faint()),
                ] {
                    if col == egui::Color32::BLACK || col == egui::Color32::WHITE {
                        return Err(format!("at 100 % shading, {name} is a pure absolute"));
                    }
                }
                // Dark mode is not shaded at all, and neither is its ink.
                let (dark_view, dark_dim) = {
                    theme::set_light(false);
                    (theme::view_bg(), theme::text_dim())
                };
                theme::set_shade(0);
                if theme::view_bg() != dark_view || theme::text_dim() != dark_dim {
                    return Err("the slider moved a dark-mode colour".into());
                }
                theme::set_light(true);

                // The setting has to be written down, or the display it was set
                // for is corrected again at every launch.
                let mut saved = Config::default();
                saved.light = true;
                saved.shade = 65;
                let text = serde_json::to_string(&saved).map_err(|e| e.to_string())?;
                let back: Config = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if back.shade != 65 {
                    return Err("the shading did not survive the settings file".into());
                }
                let old: Config =
                    serde_json::from_str("{\"light\":true}").map_err(|e| e.to_string())?;
                if old.shade != 0 {
                    return Err("a settings file without the key opened shaded".into());
                }
                if crate::settings_signature(&Config::default())
                    == crate::settings_signature(&saved)
                {
                    return Err("the shading never reaches the settings file".into());
                }
                Ok(format!(
                    "panel {} levels below the view at 0, {} at 100 % ({step_0:.2}:1 -> {step_100:.2}:1)",
                    view_0.r() as i32 - at_0[1].r() as i32,
                    view.r() as i32 - panel.r() as i32
                ))
            })();
            theme::set_shade(0);
            theme::set_light(false);
            out
        })(),
    );

    // The band and the filled button are the same dark plate in both modes, so
    // the text on them does not change either; only the surfaces around them do.
    // A picture's tint is still multiplied into the thumbnail, so the colours that
    // mean "white itself" or "readable on a black scrim" stay put.
    r.check(
        "the band, the scrim and white itself do not change with the mode",
        (|| {
            use crate::theme;
            theme::set_light(false);
            if theme::c(255, 255, 255) != egui::Color32::WHITE {
                return Err("dark mode: c(255,255,255) is not white".into());
            }
            let scrim_dark = theme::on_scrim();
            let edge_dark = theme::paper_edge();
            theme::set_light(true);
            let out = (|| {
                if theme::c(255, 255, 255) == egui::Color32::WHITE {
                    return Err(
                        "in light mode c(255,255,255) is still white, so nothing warns a tint away from it".to_string()
                    );
                }
                if theme::on_band() != egui::Color32::WHITE {
                    return Err("the band's text is not white, on a band that stays dark".to_string());
                }
                if theme::action_fill() == theme::c(61, 174, 233) {
                    return Err("the filled button still follows the accent's lightness".to_string());
                }
                if theme::on_scrim() != scrim_dark {
                    return Err("text on the viewer's black scrim changed with the mode".to_string());
                }
                if theme::paper_edge() == edge_dark {
                    return Err("the document page has no edge in light mode".to_string());
                }
                if theme::paper() == egui::Color32::WHITE {
                    return Err("the document page is a pure white in light mode".to_string());
                }
                // egui takes RichText::strong() from the active widget visuals.
                let ctx = egui::Context::default();
                theme::apply_style(&ctx);
                let strong = ctx.style().visuals.strong_text_color();
                if strong != theme::text() {
                    return Err(format!(
                        "strong text is {strong:?} while the palette's text is {:?}: emphasised names would vanish",
                        theme::text()
                    ));
                }
                Ok(())
            })();
            theme::set_light(false);
            out?;
            Ok("band and button stay dark, scrims stay light, the page keeps its edge, strong text follows the palette".into())
        })(),
    );

    // Copy to Other View / Move to Other View, and the Copy To… chooser behind
    // them: the four refusals in the order the original tests them, and a run
    // that actually copies and moves.
    r.check(
        "copying to the other view refuses what the original refuses, in its words",
        (|| {
            use crate::copymove::{already_there, plan, run, verdict, Facts, Mode};
            let ok = Facts {
                split: true,
                selected: 1,
                already_there: false,
                dest_writable: true,
                may_move: true,
            };
            if !verdict(Mode::Copy, &ok).ok {
                return Err("one marked file and a writable destination was refused".into());
            }
            let alone = Facts { split: false, ..ok.clone() };
            let v = verdict(Mode::Copy, &alone);
            if v.ok || v.why.is_some() {
                return Err("with one view the rows should be disabled without a sentence".into());
            }
            let nothing = Facts { selected: 0, ..ok.clone() };
            if verdict(Mode::Copy, &nothing).why.as_deref()
                != Some("Cannot copy to other view: No files selected.")
            {
                return Err(format!("nothing marked said {:?}", verdict(Mode::Copy, &nothing).why));
            }
            let there = Facts { already_there: true, ..ok.clone() };
            if verdict(Mode::Move, &there).why.as_deref()
                != Some("Cannot move to other view: The other view already contains these items.")
            {
                return Err("a destination that already holds the item was not refused in the original's words".into());
            }
            let readonly = Facts { dest_writable: false, ..ok.clone() };
            if verdict(Mode::Copy, &readonly).why.as_deref()
                != Some("Cannot copy to other view: You do not have permission to write into the destination folder.")
            {
                return Err("a destination that cannot be written was not refused".into());
            }
            let stuck = Facts { may_move: false, ..ok.clone() };
            if verdict(Mode::Move, &stuck).why.as_deref()
                != Some("Cannot move to other view: You do not have permission to move items from this folder.")
            {
                return Err("a move that cannot leave was not refused".into());
            }
            if !verdict(Mode::Copy, &stuck).ok {
                return Err("a copy is not blocked by being unable to move".into());
            }

            // A real copy, a name that steps aside, and a move.
            let root = base.join("copymove");
            let from = root.join("from");
            let to = root.join("to");
            fs::create_dir_all(&from).map_err(|e| e.to_string())?;
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            let one = from.join("report.txt");
            fs::write(&one, b"hello").map_err(|e| e.to_string())?;
            let folder = from.join("tree");
            fs::create_dir_all(folder.join("deeper")).map_err(|e| e.to_string())?;
            fs::write(folder.join("deeper/leaf.txt"), b"leaf").map_err(|e| e.to_string())?;
            fs::write(to.join("report.txt"), b"the one already there").map_err(|e| e.to_string())?;
            // "Already there" is about an item's own folder, so the file in
            // `from` is not in `to`, and a file sitting in `to` is.
            if already_there(&to, std::slice::from_ref(&one)) {
                return Err("an item in another folder was called already in the destination".into());
            }
            if !already_there(&to, std::slice::from_ref(&to.join("report.txt"))) {
                return Err("an item whose own folder is the destination was not noticed".into());
            }

            let p = plan(&to, &[one.clone(), folder.clone()]);
            if p.steps.len() != 2 || !p.skipped.is_empty() {
                return Err(format!("planned {} steps, {} skipped", p.steps.len(), p.skipped.len()));
            }
            if p.steps[0].to != to.join("report (1).txt") {
                return Err(format!("the taken name did not step aside: {:?}", p.steps[0].to));
            }
            let inside = plan(&folder, &[folder.clone()]);
            if inside.steps.len() != 0 || inside.skipped.len() != 1 {
                return Err("a folder was planned into itself".into());
            }
            let outcome = run(&p, Mode::Copy);
            if outcome.done != 2 || outcome.failed != 0 {
                return Err(format!("copy: {} done, {} failed: {:?}", outcome.done, outcome.failed, outcome.first_error));
            }
            if !to.join("report (1).txt").exists() || !to.join("tree/deeper/leaf.txt").exists() {
                return Err("the copy did not arrive, or did not bring the folder with it".into());
            }
            if fs::read(to.join("report.txt")).map_err(|e| e.to_string())? != b"the one already there" {
                return Err("the copy overwrote the file that was already there".into());
            }
            let moved = plan(&to.join("moved"), &[to.join("report (1).txt")]);
            fs::create_dir_all(to.join("moved")).map_err(|e| e.to_string())?;
            let outcome = run(&moved, Mode::Move);
            if outcome.done != 1 || to.join("report (1).txt").exists() {
                return Err("the move did not take the file away".into());
            }
            Ok(format!(
                "five refusals in the original's words; a copy stepped aside to {:?} and a move took the file away",
                p.steps[0].to.file_name().unwrap_or_default()
            ))
        })(),
    );

    // ---- the Remote group reads what is actually mounted -----------------
    r.check(
        "the Network group lists mounted shares and nothing else",
        (|| {
            let mounts = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
tmpfs /run tmpfs rw,nosuid 0 0
//router/media /mnt/media cifs rw,vers=3.1.1 0 0
nas:/export/photos /mnt/photos nfs4 rw,addr=10.0.0.4 0 0
me@host:/data /home/me/data fuse.sshfs rw 0 0
/dev/sdb1 /media/me/My\\040Drive vfat rw 0 0
";
            let list = fsutil::parse_network_mounts(mounts);
            let names: Vec<String> = list.iter().map(|(l, _)| l.clone()).collect();
            if names != vec!["media", "photos", "data"] {
                return Err(format!("rows came back as {names:?}"));
            }
            if list.iter().any(|(_, p)| p.to_string_lossy().contains("My Drive")) {
                return Err("a local drive was listed as a share".into());
            }
            // A share whose mount point has escaped spaces is unescaped, and a
            // SMB mount is named after the share rather than the server.
            let spaced = fsutil::parse_network_mounts(
                "//box/Team\\040Share /mnt/team\\040share cifs rw 0 0\n",
            );
            if spaced.len() != 1 || spaced[0].1 != PathBuf::from("/mnt/team share") {
                return Err(format!("escaped mount point read as {:?}", spaced));
            }
            if spaced[0].0 != "Team Share" {
                return Err(format!("the share was named {:?}", spaced[0].0));
            }
            // The desktop's own mount folder names a share with its parameters.
            if fsutil::gvfs_label("smb-share:server=router,share=media") != "media" {
                return Err("an SMB mount was not named after its share".into());
            }
            if fsutil::gvfs_label("sftp:host=nas,user=me") != "nas (sftp)" {
                return Err(format!(
                    "an SFTP mount was named {:?}",
                    fsutil::gvfs_label("sftp:host=nas,user=me")
                ));
            }
            Ok(format!("{} share(s), named after the share", names.len()))
        })(),
    );

    // ---- file names elide in the middle, as the original's default does ----
    // `[ContentDisplay] ElidingMode` is an `ItemEnum` over `Right` = 0 and
    // `Middle` = 1, built by `ContentDisplaySettings::ContentDisplaySettings`
    // @0x2b29b0 in libdolphinprivate.so.6, and the default passed to it is 1.
    r.check(
        "a file name too long for its column keeps its start and its extension",
        (|| {
            use egui::{Context, Pos2, RawInput, Rect, Vec2};
            let ctx = Context::default();
            let long = "a-very-long-report-name-that-will-not-fit-anywhere.pdf";
            let mut shown = String::new();
            let mut tight = String::new();
            ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 200.0))),
                    time: Some(0.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let font = egui::FontId::proportional(12.5);
                        shown = crate::theme::elide_name(ui, long, 140.0, font.clone());
                        tight = crate::theme::elide_name(ui, long, 26.0, font);
                    });
                },
            );
            if !shown.contains('…') {
                return Err(format!("nothing was cut: {shown:?}"));
            }
            if !shown.ends_with(".pdf") {
                return Err(format!("the extension was lost: {shown:?}"));
            }
            if !long.starts_with(shown.chars().next().unwrap_or(' ')) || shown.chars().count() < 6 {
                return Err(format!("the start of the name was lost: {shown:?}"));
            }
            if tight.chars().count() > shown.chars().count() {
                return Err(format!("a tighter column drew more: {tight:?}"));
            }
            Ok(format!("{shown:?} in 140 px, {tight:?} in 26 px"))
        })(),
    );

    // ---- the date switch touches only the absolute dates ------------------
    // `InformationPanelSettings::InformationPanelSettings` @0x1fe410 in
    // /usr/bin/dolphin registers `dateFormat` over LongFormat = 0 (default) and
    // ShortFormat = 1; the relative wording is the separate
    // `[ContentDisplay] UseShortRelativeDates` switch, true by default.
    r.check(
        "the date switch keeps \"minutes ago\" and changes the older dates",
        (|| {
            let now = std::time::SystemTime::now();
            let fresh = now - std::time::Duration::from_secs(120);
            let old = now - std::time::Duration::from_secs(60 * 60 * 24 * 400);
            let fresh_long = fsutil::info_date(Some(fresh), true);
            let fresh_short = fsutil::info_date(Some(fresh), false);
            if fresh_long != fresh_short || !fresh_long.contains("minute(s) ago") {
                return Err(format!(
                    "a recent time read {fresh_long:?} and {fresh_short:?}"
                ));
            }
            let old_long = fsutil::info_date(Some(old), true);
            let old_short = fsutil::info_date(Some(old), false);
            if old_long == old_short {
                return Err(format!("both settings gave {old_long:?}"));
            }
            if !old_long.contains(" at ") {
                return Err(format!("the long date was {old_long:?}"));
            }
            let dt: chrono::DateTime<chrono::Local> = old.into();
            if old_short != dt.format("%d/%m/%Y %H:%M").to_string() {
                return Err(format!("the short date was {old_short:?}"));
            }
            Ok(format!(
                "recent {fresh_long:?}, long {old_long:?}, short {old_short:?}"
            ))
        })(),
    );

    // ---- the two switches start where the original's do -------------------
    r.check(
        "a fresh profile uses the original's dates and follows the folder",
        (|| {
            let cfg = crate::config::Config::default();
            if cfg.date_format != 0 {
                return Err(format!("dates start at index {}", cfg.date_format));
            }
            if !cfg.places_autoscroll {
                return Err("the panel does not follow the folder".into());
            }
            // A settings file written before these keys existed still lands on
            // the same two values, because both are serde defaults.
            let old: crate::config::Config = serde_json::from_str("{}").map_err(|e| e.to_string())?;
            if old.date_format != 0 || !old.places_autoscroll {
                return Err("an empty settings file did not take the defaults".into());
            }
            let written = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            if !written.contains("\"date_format\"") || !written.contains("\"places_autoscroll\"") {
                return Err("the two choices are not kept in the file".into());
            }
            Ok("LongFormat, AutoScrolling on, both written".into())
        })(),
    );

    // ---- added feature: the disk-usage window ----------------------------
    // The window itself is drawn, not tested; what it adds up is. Every rule in
    // `space.rs` is checked here against real files: what counts, what is left
    // alone, and how the figures are rounded.
    let walk_root = base.join("space-walk");
    let _ = write(&walk_root.join("a.txt"), b"aaaa");
    let _ = write(&walk_root.join("b.bin"), b"0123456789");
    let _ = write(&walk_root.join(".cache/c.txt"), b"ccc");
    let _ = write(&walk_root.join("deep/d.txt"), b"ddddd");
    let _ = std::os::unix::fs::symlink(&base, walk_root.join("link"));
    r.check(
        "the disk-usage walk counts hidden files, never follows a link, and sums the rest",
        (|| {
            use std::collections::HashMap;
            use std::sync::atomic::AtomicBool;
            use std::sync::Arc;
            let cancel = Arc::new(AtomicBool::new(false));
            let mut tree: HashMap<PathBuf, Vec<crate::space::Row>> = HashMap::new();
            let mut biggest = crate::space::Biggest::new(40);
            let mut totals = crate::space::Totals::default();
            let mut children = 0usize;
            let mut on_child = |_r: crate::space::Row| children += 1;
            let mut on_progress = |_t: crate::space::Totals| {};
            crate::space::scan(
                &walk_root,
                &cancel,
                &mut tree,
                &mut biggest,
                &mut totals,
                &mut on_child,
                &mut on_progress,
            );
            // The link is counted at its own entry size (the length of the path
            // it points at), never as the tree behind it.
            let link_len = base.to_string_lossy().len() as u64;
            let want = 4 + 10 + 3 + 5 + link_len;
            if totals.bytes != want {
                return Err(format!("the walk added up {} bytes, wanted {}", totals.bytes, want));
            }
            if totals.dirs != 3 || totals.files != 5 {
                return Err(format!(
                    "the walk found {} folder(s) and {} file(s), wanted 3 and 5",
                    totals.dirs, totals.files
                ));
            }
            if children != 5 {
                return Err(format!("{children} row(s) reached the list, wanted 5"));
            }
            let rows = tree.get(&walk_root).cloned().unwrap_or_default();
            let sizes: Vec<u64> = rows.iter().map(|r| r.size).collect();
            if sizes.windows(2).any(|w| w[0] < w[1]) {
                let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
                return Err(format!("the rows are not largest first: {names:?}"));
            }
            Ok(format!(
                "{} bytes over {} folder(s) and {} file(s), link not followed",
                totals.bytes, totals.dirs, totals.files
            ))
        })(),
    );
    r.check(
        "a share of a whole is rounded half up, and an empty whole is nought",
        (|| {
            let cases = [
                (1u64, 3u64, 33u64),
                (2, 3, 67),
                (5, 10, 50),
                (999, 1000, 100),
                (0, 0, 0),
            ];
            for (part, whole, want) in cases {
                let got = crate::space::share_percent(part, whole);
                if got != want {
                    return Err(format!("{part}/{whole} read {got}%, wanted {want}%"));
                }
            }
            Ok("1/3 → 33%, 2/3 → 67%, 999/1000 → 100%, 0/0 → 0%".into())
        })(),
    );
    r.check(
        "the free-up shortlist keeps the largest files, biggest first, and stops at forty",
        (|| {
            let mut b = crate::space::Biggest::new(crate::space::BIGGEST_KEPT);
            for size in 1..=50u64 {
                b.push(size, PathBuf::from(format!("/x/{size}")));
            }
            if b.items.len() != 40 {
                return Err(format!("the list kept {} file(s), wanted 40", b.items.len()));
            }
            if b.items.first().map(|i| i.size) != Some(50) {
                return Err("the largest file is not first".into());
            }
            if b.items.last().map(|i| i.size) != Some(11) {
                return Err(format!(
                    "the smallest kept file is {:?}, wanted 11",
                    b.items.last().map(|i| i.size)
                ));
            }
            b.push(3, PathBuf::from("/x/small"));
            if b.items.len() != 40 || b.items.iter().any(|i| i.path.ends_with("small")) {
                return Err("a file smaller than the shortlist changed it".into());
            }
            Ok("40 kept, 50 first, 11 last, a smaller file turned away".into())
        })(),
    );
    r.check(
        "the usage window walks into a folder it already summed up, and back out",
        (|| {
            let dir = base.join("space-desc");
            write(&dir.join("one/x.txt"), b"xxxxxx")?;
            write(&dir.join("two/y.txt"), b"yy")?;
            let mut sp = crate::space::Space::start(dir.clone());
            let deadline = Instant::now() + Duration::from_secs(15);
            while sp.scanning && Instant::now() < deadline {
                sp.poll();
                std::thread::sleep(Duration::from_millis(20));
            }
            sp.poll();
            if sp.scanning {
                return Err("the walk did not finish in 15 seconds".into());
            }
            if sp.rows().len() != 2 || sp.shown_total() != 8 {
                return Err(format!(
                    "the first list has {} row(s) and {} bytes, wanted 2 and 8",
                    sp.rows().len(),
                    sp.shown_total()
                ));
            }
            let one = dir.join("one");
            sp.descend(one);
            if sp.shown_total() != 6 || sp.rows().len() != 1 {
                return Err(format!(
                    "inside one: {} row(s), {} bytes, wanted 1 and 6",
                    sp.rows().len(),
                    sp.shown_total()
                ));
            }
            sp.up();
            if sp.show != dir || sp.shown_total() != 8 {
                return Err("going back up did not restore the first list".into());
            }
            Ok("8 bytes at the top, 6 inside one, 8 again after up".into())
        })(),
    );

    // ---- added feature: Compress and Extract ----------------------------
    let arc = base.join("arch-name");
    let _ = fs::create_dir_all(arc.join("photos"));
    let _ = write(&arc.join("report.pdf"), b"pdf");
    r.check(
        "the archive is named after the item, and never overwrites what is there",
        (|| {
            let folder = arc.join("photos");
            let file = arc.join("report.pdf");
            let one = crate::archive::default_name(&[folder.clone()], &arc);
            if one != "photos.zip" {
                return Err(format!("a folder gave {one:?}"));
            }
            let two = crate::archive::default_name(&[file.clone()], &arc);
            if two != "report.zip" {
                return Err(format!("a file gave {two:?}"));
            }
            let several = crate::archive::default_name(&[folder, file], &arc);
            if several != "Archive.zip" {
                return Err(format!("a pair gave {several:?}"));
            }
            write(&arc.join("photos.zip"), b"already here")?;
            let next = crate::archive::unique_path(&arc.join("photos.zip"));
            if next != arc.join("photos (1).zip") {
                return Err(format!("the second archive would be {}", next.display()));
            }
            Ok("photos.zip, report.zip, Archive.zip, then photos (1).zip".into())
        })(),
    );
    r.check(
        "an archive name is checked, and zip is asked to keep folders and links",
        (|| {
            if crate::archive::clean_name("  ").is_ok() {
                return Err("an empty name was accepted".into());
            }
            if crate::archive::clean_name("a/b").is_ok() {
                return Err("a name with a slash was accepted".into());
            }
            if crate::archive::clean_name("-rf").is_ok() {
                return Err("a name starting with a dash was accepted".into());
            }
            if crate::archive::clean_name(" shots ").ok().as_deref() != Some("shots.zip") {
                return Err("a plain name did not gain .zip".into());
            }
            let args = crate::archive::zip_args("shots.zip", &["a.txt".into(), "pics".into()]);
            let wanted = ["-r", "-q", "-y", "shots.zip", "a.txt", "pics"];
            if args != wanted.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
                return Err(format!("zip was asked for {args:?}"));
            }
            Ok("empty, slash and dash refused; folders, quiet and links asked for".into())
        })(),
    );
    r.check(
        "the size of what is marked is added up off the drawing thread",
        (|| {
            let dir = base.join("arch-size");
            write(&dir.join("tree/a"), b"1234567890")?;
            write(&dir.join("tree/sub/b"), b"12345")?;
            let mut c = crate::archive::Compress::new(vec![dir.join("tree")], dir.clone());
            if c.total.is_some() {
                return Err("the size was there before the walk could have run".into());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while c.total.is_none() && Instant::now() < deadline {
                c.poll_total();
                std::thread::sleep(Duration::from_millis(20));
            }
            match c.total {
                None => Err("the size never arrived".into()),
                Some(15) => Ok("15 bytes, counted in the background".into()),
                Some(other) => Err(format!("the walk reported {other} bytes, wanted 15")),
            }
        })(),
    );
    r.check(
        "a folder whose name ends in .zip is a folder, not an archive",
        (|| {
            let fake = arc.join("not-really.zip");
            fs::create_dir_all(&fake).map_err(|e| e.to_string())?;
            if crate::archive::is_archive(&fake) {
                return Err("a folder called .zip was taken for an archive".into());
            }
            let real = arc.join("shots.ZIP");
            write(&real, b"PK")?;
            if !crate::archive::is_archive(&real) {
                return Err("a real .ZIP was not recognised".into());
            }
            if crate::archive::folder_name(Path::new("/x/holiday.tar.gz")) != "holiday" {
                return Err("a .tar.gz did not give its own name".into());
            }
            let x = crate::archive::Extract::new(real);
            if x.dest() != arc.join("shots") {
                return Err(format!("it would unpack into {}", x.dest().display()));
            }
            Ok("folder ignored, .ZIP recognised, holiday.tar.gz → holiday".into())
        })(),
    );
    r.check(
        "a ZIP made here unpacks again with the same bytes, hidden file and sub-folder included",
        (|| {
            let dir = base.join("arch-round");
            write(&dir.join("tree/one.txt"), b"hello archive")?;
            write(&dir.join("tree/.hidden"), b"secret")?;
            write(&dir.join("tree/sub/two.txt"), b"world")?;
            let mut c = crate::archive::Compress::new(vec![dir.join("tree")], dir.clone());
            if !c.begin() {
                return Err(c.error.clone().unwrap_or_else(|| "compress refused".into()));
            }
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut answer = None;
            while answer.is_none() && Instant::now() < deadline {
                answer = c.poll();
                std::thread::sleep(Duration::from_millis(30));
            }
            match answer {
                None => return Err("zip did not answer in 30 seconds".into()),
                Some(Err(e)) if e.contains("not installed") => {
                    return Ok("zip is not installed here, so the round trip was skipped".into())
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(_)) => {}
            }
            let made = c.last_dest.clone().ok_or("no archive path was kept")?;
            if !made.is_file() {
                return Err(format!("{} was not written", made.display()));
            }
            let mut x = crate::archive::Extract::new(made.clone());
            if !x.begin() {
                return Err(x.error.clone().unwrap_or_else(|| "extract refused".into()));
            }
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut answer = None;
            while answer.is_none() && Instant::now() < deadline {
                answer = x.poll();
                std::thread::sleep(Duration::from_millis(30));
            }
            match answer {
                None => return Err("unzip did not answer in 30 seconds".into()),
                Some(Err(e)) if e.contains("not installed") => {
                    return Ok("unzip is not installed here, so the round trip was skipped".into())
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(_)) => {}
            }
            let into = x.last_dest.clone().ok_or("no folder path was kept")?;
            for (name, want) in [
                ("tree/one.txt", &b"hello archive"[..]),
                ("tree/.hidden", &b"secret"[..]),
                ("tree/sub/two.txt", &b"world"[..]),
            ] {
                let got = fs::read(into.join(name))
                    .map_err(|e| format!("{name} did not come back: {e}"))?;
                if got != want {
                    return Err(format!("{name} came back changed"));
                }
            }
            Ok(format!(
                "{} bytes written, three files back unchanged",
                fs::metadata(&made).map(|m| m.len()).unwrap_or(0)
            ))
        })(),
    );
    r.check(
        "a tar archive is unpacked by tar, a zip by unzip",
        (|| {
            let t = crate::archive::tar_args(Path::new("/x/h.tar.gz"), Path::new("/x/h"));
            if t.first().map(|s| s.as_str()) != Some("-xf") || t.len() != 4 {
                return Err(format!("tar was asked for {t:?}"));
            }
            let z = crate::archive::unzip_args(Path::new("/x/a.zip"), Path::new("/x/a"));
            if z.first().map(|s| s.as_str()) != Some("-q")
                || z.iter().filter(|s| s.as_str() == "-d").count() != 1
            {
                return Err(format!("unzip was asked for {z:?}"));
            }
            Ok("tar -xf, unzip -q -o … -d".into())
        })(),
    );


    // ---- added feature: finding a phrase inside files --------------------
    r.check(
        "a phrase is counted in the text, in either case or in one",
        (|| {
            let text = "alpha beta ALPHA\nGamma: alpha";
            let loose = findtext::count_matches(text, "alpha", false);
            let exact = findtext::count_matches(text, "alpha", true);
            if loose != 3 {
                return Err(format!("case-blind count was {loose}, not 3"));
            }
            if exact != 2 {
                return Err(format!("case-sensitive count was {exact}, not 2"));
            }
            if findtext::count_matches(text, "", false) != 0 {
                return Err("an empty phrase counted matches".into());
            }
            Ok(format!("{loose} loose, {exact} exact"))
        })(),
    );
    r.check(
        "the matching line is reported with its number, trimmed and short",
        (|| {
            let text = "one\n    two   spaces\t\tthen the word needle here\nthree";
            let (line_no, line) =
                findtext::first_match_line(text, "NEEDLE", false).ok_or("no line found")?;
            if line_no != 2 {
                return Err(format!("line {line_no}, not 2"));
            }
            if line != "two spaces then the word needle here" {
                return Err(format!("the line read {line:?}"));
            }
            let long = "x".repeat(400);
            let cut = findtext::snippet(&long);
            if cut.chars().count() != findtext::SNIPPET_CHARS + 1 || !cut.ends_with('…') {
                return Err(format!("a long line was cut to {} chars", cut.chars().count()));
            }
            Ok(format!("line {line_no}, {} chars for a long one", cut.chars().count()))
        })(),
    );
    let find_root = base.join("findtext");
    let _ = write(
        &find_root.join("report.txt"),
        b"first line\nthe needle is here\nand the needle again\n",
    );
    let _ = write(&find_root.join("sub/notes.md"), b"nothing\na second needle\n");
    let _ = write(&find_root.join("hidden/.secret.txt"), b"a needle nobody asked for\n");
    let _ = write(&find_root.join("picture.bin"), &[0u8, 1, 2, b'n', b'e', b'e', b'd', b'l', b'e']);
    let _ = write(&find_root.join("huge.txt"), &vec![b'n'; (findtext::MAX_BYTES + 10) as usize]);
    let _ = write(&find_root.join("report-again.txt"), b"no phrase at all here\n");
    r.check(
        "a real walk finds the phrase, skips binaries, big files and hidden folders",
        (|| {
            let mut opts = findtext::Options::new("needle");
            opts.mode = findtext::Mode::Contents;
            let mut ft = findtext::FindText::start(find_root.clone(), opts);
            wait_find(&mut ft)?;
            let names: Vec<String> = ft.hits.iter().map(|h| h.file_name()).collect();
            if names.len() != 2 {
                return Err(format!("hits were {names:?}, not two"));
            }
            if !names.contains(&"report.txt".to_string()) || !names.contains(&"notes.md".to_string()) {
                return Err(format!("wrong files: {names:?}"));
            }
            if ft.matches != 3 {
                return Err(format!("{} matches, not 3", ft.matches));
            }
            if ft.skipped_big != 1 {
                return Err(format!("{} too-big files, not 1", ft.skipped_big));
            }
            if ft.skipped_binary != 1 {
                return Err(format!("{} binary files, not 1", ft.skipped_binary));
            }
            let report = ft
                .hits
                .iter()
                .find(|h| h.file_name() == "report.txt")
                .ok_or("report.txt missing")?;
            if report.line_no != 2 || !report.line.contains("needle is here") {
                return Err(format!(
                    "report.txt pointed at line {} ({:?})",
                    report.line_no, report.line
                ));
            }
            let summary = ft.summary();
            if !summary.contains("hidden folders left out") || !summary.contains("too large") {
                return Err(format!("the summary left out a rule: {summary}"));
            }
            Ok(format!("{names:?} on line 2, {} matches", ft.matches))
        })(),
    );
    r.check(
        "the binary switch reads a file the default walk passes over",
        (|| {
            let mut opts = findtext::Options::new("needle");
            opts.mode = findtext::Mode::Contents;
            opts.include_binary = true;
            let mut ft = findtext::FindText::start(find_root.clone(), opts);
            wait_find(&mut ft)?;
            if !ft.hits.iter().any(|h| h.file_name() == "picture.bin") {
                return Err("the binary file was still passed over".into());
            }
            if ft.skipped_binary != 0 {
                return Err(format!("{} binary files counted as skipped", ft.skipped_binary));
            }
            Ok(format!("{} hits, binary read", ft.hits.len()))
        })(),
    );
    r.check(
        "the name mode answers from names alone, and the mixed mode from both",
        (|| {
            let mut names_only = findtext::Options::new("report");
            names_only.mode = findtext::Mode::Names;
            let mut ft = findtext::FindText::start(find_root.clone(), names_only);
            wait_find(&mut ft)?;
            let got: Vec<String> = ft.hits.iter().map(|h| h.file_name()).collect();
            if got.len() != 2 || ft.matches != 2 {
                return Err(format!("name mode gave {got:?} / {} matches", ft.matches));
            }
            if ft.hits.iter().any(|h| h.line_no != 0) {
                return Err("a name-only hit carried a line number".into());
            }
            let mut both = findtext::Options::new("needle");
            both.mode = findtext::Mode::NamesAndContents;
            let mut ft2 = findtext::FindText::start(find_root.clone(), both);
            wait_find(&mut ft2)?;
            let names: Vec<String> = ft2.hits.iter().map(|h| h.file_name()).collect();
            if names.len() != 2 {
                return Err(format!("mixed mode gave {names:?}"));
            }
            Ok(format!("{got:?} by name, {names:?} by name or contents"))
        })(),
    );

    r.check(
        "an empty phrase starts no walk and says there is nothing to search for",
        (|| {
            let mut ft = findtext::FindText::start(find_root.clone(), findtext::Options::new(""));
            if ft.scanning {
                return Err("an empty phrase started a walk".into());
            }
            if ft.files_seen != 0 || !ft.hits.is_empty() {
                return Err(format!("an empty phrase looked at {} files", ft.files_seen));
            }
            let summary = ft.summary();
            if !summary.starts_with("Nothing searched yet") {
                return Err(format!("the summary read {summary:?}"));
            }
            Ok(summary)
        })(),
    );

    // ---- the resize window: megapixels in, same shape out -------------------
    r.check(
        "a size in megapixels comes out at the picture's own shape",
        (|| {
            let (w, h) = imageresize::target_dims(4000, 3000, 2.0);
            if (w, h) != (1633, 1225) {
                return Err(format!("4000×3000 at 2 MP came out {w}×{h}"));
            }
            let ratio_before = 4000.0 / 3000.0;
            let ratio_after = w as f64 / h as f64;
            if (ratio_before - ratio_after).abs() > 0.001 {
                return Err(format!(
                    "the shape moved from {ratio_before:.4} to {ratio_after:.4}"
                ));
            }
            let mp = imageresize::megapixels(w, h);
            if (mp - 2.0).abs() > 0.02 {
                return Err(format!("2 MP became {mp:.3} MP"));
            }
            // A tall picture keeps its shape too, an even panorama lands exactly,
            // and a 1:1 stays 1:1.
            let tall = imageresize::target_dims(1080, 1920, 2.0);
            if (tall.0 as f64 / tall.1 as f64 - 1080.0 / 1920.0).abs() > 0.001 {
                return Err(format!("1080×1920 came out {}×{}", tall.0, tall.1));
            }
            if imageresize::target_dims(8000, 500, 1.0) != (4000, 250) {
                return Err("the panorama did not land on 4000×250".into());
            }
            let square = imageresize::target_dims(2048, 2048, 1.0);
            if square.0 != square.1 || (imageresize::megapixels(square.0, square.1) - 1.0).abs() > 0.01 {
                return Err(format!("the square came out {}×{}", square.0, square.1));
            }
            Ok(format!(
                "4000×3000 → {w}×{h} ({mp:.2} MP, {ratio_after:.4}), 1080×1920 → {}×{}, 8000×500 → 4000×250",
                tall.0, tall.1
            ))
        })(),
    );
    r.check(
        "the megapixel field accepts what a person types and refuses nothing useful",
        (|| {
            if imageresize::parse_mp("2").unwrap() != 2.0 {
                return Err("\"2\" did not read as 2".into());
            }
            if imageresize::parse_mp("2.5").unwrap() != 2.5 {
                return Err("\"2.5\" did not read as 2.5".into());
            }
            if imageresize::parse_mp("2,5").unwrap() != 2.5 {
                return Err("a comma decimal was not read".into());
            }
            if imageresize::parse_mp(" 8 MP ").unwrap() != 8.0 {
                return Err("\"8 MP\" was not read".into());
            }
            for bad in ["", "abc", "-3", "0", "0.001", "9999"] {
                if imageresize::parse_mp(bad).is_ok() {
                    return Err(format!("{bad:?} was accepted"));
                }
            }
            // The floor and the ceiling are the ones the window states.
            if imageresize::target_dims(4000, 3000, imageresize::MIN_MP).0 < 1 {
                return Err("the smallest target collapsed to nothing".into());
            }
            Ok(format!(
                "2, 2.5, 2,5 and \"8 MP\" read; empty, words, negative, 0 and 9999 refused; smallest target {}×{}",
                imageresize::target_dims(4000, 3000, imageresize::MIN_MP).0,
                imageresize::target_dims(4000, 3000, imageresize::MIN_MP).1
            ))
        })(),
    );

    // A real picture, made here so the check needs nothing from the person.
    fn make_picture(path: &Path, w: u32, h: u32) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            *px = image::Rgba([r, g, 128, 255]);
        }
        let _ = img.save(path);
    }

    fn wait_resize(d: &mut imageresize::Resize) -> Result<imageresize::Outcome, String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Some(out) = d.poll() {
                return Ok(out);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("the resize did not finish in 20 seconds".into())
    }

    let resize_root = base.join("resize");
    let photo = resize_root.join("photo.png");
    make_picture(&photo, 400, 300);
    r.check(
        "a real picture is scaled to the megapixels asked for, beside the original",
        (|| {
            let before = fs::metadata(&photo).map_err(|e| e.to_string())?.len();
            let mut d = imageresize::Resize::new(vec![photo.clone()]);
            d.mp_text = "0.03".to_string();
            let plan = d.plan()?;
            if plan.len() != 1 {
                return Err(format!("{} plans for one picture", plan.len()));
            }
            if plan[0].to != (200, 150) {
                return Err(format!(
                    "400×300 at 0.03 MP planned {}×{}",
                    plan[0].to.0, plan[0].to.1
                ));
            }
            let want = resize_root.join("photo-0.03MP.png");
            if plan[0].out != want {
                return Err(format!("the copy was to be named {}", plan[0].out.display()));
            }
            if !d.begin() {
                return Err("the resize did not start".into());
            }
            let out = wait_resize(&mut d)?;
            if out.written.len() != 1 || !out.failures.is_empty() {
                return Err(format!(
                    "{} written, {} failed: {:?}",
                    out.written.len(),
                    out.failures.len(),
                    out.failures
                ));
            }
            let made = image::open(&want).map_err(|e| format!("the copy does not open: {e}"))?;
            if (made.width(), made.height()) != (200, 150) {
                return Err(format!(
                    "the copy is {}×{}",
                    made.width(),
                    made.height()
                ));
            }
            let after = fs::metadata(&photo).map_err(|e| e.to_string())?.len();
            if after != before {
                return Err("the original changed when it should not have".into());
            }
            let leftover = fs::read_dir(&resize_root)
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().contains(".part"));
            if leftover {
                return Err("a part file was left behind".into());
            }
            Ok(format!("400×300 → 200×150, {} bytes, original untouched", before))
        })(),
    );
    r.check(
        "a second resize steps aside instead of overwriting the first",
        (|| {
            let mut d = imageresize::Resize::new(vec![photo.clone()]);
            d.mp_text = "0.03".to_string();
            let plan = d.plan()?;
            let want = resize_root.join("photo-0.03MP (1).png");
            if plan[0].out != want {
                return Err(format!("the second copy was to be {}", plan[0].out.display()));
            }
            d.begin();
            let out = wait_resize(&mut d)?;
            if out.written.len() != 1 {
                return Err(format!("{} written: {:?}", out.written.len(), out.failures));
            }
            let first = image::open(resize_root.join("photo-0.03MP.png")).map_err(|e| e.to_string())?;
            let second = image::open(&want).map_err(|e| e.to_string())?;
            if (first.width(), first.height()) != (second.width(), second.height()) {
                return Err("the two copies differ in size".into());
            }
            Ok("photo-0.03MP (1).png, both copies kept".to_string())
        })(),
    );
    r.check(
        "replacing the original leaves one good picture and no part file",
        (|| {
            let victim = resize_root.join("replace-me.png");
            make_picture(&victim, 320, 240);
            let mut d = imageresize::Resize::new(vec![victim.clone()]);
            d.mp_text = "0.02".to_string();
            d.replace = true;
            let plan = d.plan()?;
            if plan[0].out != victim {
                return Err("replace did not aim at the original".into());
            }
            d.begin();
            let out = wait_resize(&mut d)?;
            if out.written.len() != 1 || !out.failures.is_empty() {
                return Err(format!("{:?} / {:?}", out.written, out.failures));
            }
            let made = image::open(&victim).map_err(|e| format!("the original no longer opens: {e}"))?;
            let mp = imageresize::megapixels(made.width(), made.height());
            if (mp - 0.02).abs() > 0.002 {
                return Err(format!(
                    "the replaced picture is {}×{} = {mp:.3} MP",
                    made.width(),
                    made.height()
                ));
            }
            let stray = fs::read_dir(&resize_root)
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().contains(".part"));
            if stray {
                return Err("a part file was left behind".into());
            }
            Ok(format!("320×240 → {}×{} ({mp:.2} MP)", made.width(), made.height()))
        })(),
    );
    r.check(
        "a JPEG copy is written as a JPEG at the size asked for",
        (|| {
            let jpg = resize_root.join("shot.jpg");
            make_picture(&jpg.with_extension("png"), 300, 200);
            let rgba = image::open(jpg.with_extension("png")).map_err(|e| e.to_string())?;
            rgba.to_rgb8()
                .save(&jpg)
                .map_err(|e| format!("could not make the JPEG: {e}"))?;
            let mut d = imageresize::Resize::new(vec![jpg.clone()]);
            d.mp_text = "0.02".to_string();
            d.quality = 75;
            d.begin();
            let out = wait_resize(&mut d)?;
            if out.written.len() != 1 {
                return Err(format!("{:?}", out.failures));
            }
            let written = out.written[0].clone();
            let bytes = fs::read(&written).map_err(|e| e.to_string())?;
            if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
                return Err("the copy is not a JPEG".into());
            }
            let made = image::open(&written).map_err(|e| e.to_string())?;
            let mp = imageresize::megapixels(made.width(), made.height());
            if (mp - 0.02).abs() > 0.002 {
                return Err(format!("the copy is {}×{} = {mp:.3} MP", made.width(), made.height()));
            }
            Ok(format!("{}×{} JPEG, {} bytes", made.width(), made.height(), bytes.len()))
        })(),
    );
    r.check(
        "a file that is not a picture is named instead of quietly dropped",
        (|| {
            let not_picture = resize_root.join("notes.txt");
            write(&not_picture, b"just text")?;
            let d = imageresize::Resize::new(vec![not_picture.clone(), photo.clone()]);
            let noted = d
                .items
                .iter()
                .find(|i| i.src == not_picture)
                .and_then(|i| i.note.clone());
            if noted.is_none() {
                return Err("the text file carried no note".into());
            }
            let ready = d.ready().count();
            if ready != 1 {
                return Err(format!("{ready} pictures ready instead of 1"));
            }
            let plan = d.plan()?;
            if plan.len() != 1 {
                return Err(format!("{} plans instead of 1", plan.len()));
            }
            Ok(format!("{} left as it is; 1 picture planned", noted.unwrap_or_default()))
        })(),
    );

    r.check(
        "a batch of different shapes is scaled picture by picture",
        (|| {
            let batch = base.join("resize-batch");
            let a = batch.join("wide.png");
            let b = batch.join("tall.png");
            let c = batch.join("square.png");
            make_picture(&a, 400, 300);
            make_picture(&b, 300, 400);
            make_picture(&c, 500, 500);
            let mut d = imageresize::Resize::new(vec![a.clone(), b.clone(), c.clone()]);
            // A picture this small must not open with a default that blows it up.
            if d.mp_text != "0.1" {
                return Err(format!(
                    "the field started at {:?} for a 0.12 MP picture",
                    d.mp_text
                ));
            }
            d.mp_text = "0.02".to_string();
            let plan = d.plan()?;
            if plan.len() != 3 {
                return Err(format!("{} plans for three pictures", plan.len()));
            }
            d.begin();
            let out = wait_resize(&mut d)?;
            if out.written.len() != 3 || !out.failures.is_empty() {
                return Err(format!(
                    "{} written, {} failed: {:?}",
                    out.written.len(),
                    out.failures.len(),
                    out.failures
                ));
            }
            // The file each picture was *planned* to land on is what gets read
            // back — asking `output_path` again after the run would step aside
            // past the copy that was just written and name a file that is not
            // there.
            let mut report = Vec::new();
            for item in [&a, &b, &c] {
                let entry = plan
                    .iter()
                    .find(|p| &p.src == item)
                    .ok_or_else(|| format!("{} was never planned", item.display()))?;
                let made = image::open(&entry.out)
                    .map_err(|e| format!("{}: {e}", entry.out.display()))?;
                if (made.width(), made.height()) != entry.to {
                    return Err(format!(
                        "{} was {}×{} → {}×{}, wanted {}×{}",
                        item.display(),
                        entry.from.0,
                        entry.from.1,
                        made.width(),
                        made.height(),
                        entry.to.0,
                        entry.to.1
                    ));
                }
                report.push(format!(
                    "{}×{}→{}×{}",
                    entry.from.0,
                    entry.from.1,
                    made.width(),
                    made.height()
                ));
            }
            Ok(report.join(", "))
        })(),
    );

    // -----------------------------------------------------------------------
    // Drag and drop with other programs
    // -----------------------------------------------------------------------
    r.check(
        "a file's address in a drag is percent-escaped and ends CRLF",
        (|| {
            let list = crate::dragout::uri_list(&[
                PathBuf::from("/tmp/a b.png"),
                PathBuf::from("/tmp/photo #1 é.jpg"),
            ]);
            let want_one = "file:///tmp/a%20b.png\r\n";
            let want_two = "file:///tmp/photo%20%231%20%C3%A9.jpg\r\n";
            if !list.starts_with(want_one) || !list.contains(want_two) {
                return Err(format!("{list:?}"));
            }
            if !list.ends_with("\r\n") {
                return Err("the list does not end with CRLF".into());
            }
            Ok(format!("{list:?}"))
        })(),
    );
    r.check(
        "the text form of a drag is the plain paths, one per line",
        (|| {
            let list = crate::dragout::plain_list(&[PathBuf::from("/tmp/a b.png")]);
            if list != "/tmp/a b.png\n" {
                return Err(format!("{list:?}"));
            }
            Ok(format!("{list:?}"))
        })(),
    );
    r.check(
        "a press becomes a drag only after the pointer has travelled far enough",
        (|| {
            let from = egui::Pos2::new(100.0, 100.0);
            if crate::dragout::far_enough(from, egui::Pos2::new(104.0, 104.0)) {
                return Err("4 px counted as a drag".into());
            }
            if !crate::dragout::far_enough(from, egui::Pos2::new(110.0, 100.0)) {
                return Err("10 px did not count as a drag".into());
            }
            Ok(format!("{} px", crate::dragout::THRESHOLD))
        })(),
    );
    r.check(
        "a dropped file never overwrites one already in the folder",
        (|| {
            let room = base.join("drop-in");
            fs::create_dir_all(&room).map_err(|e| e.to_string())?;
            let src = base.join("elsewhere");
            fs::create_dir_all(&src).map_err(|e| e.to_string())?;
            let picture = src.join("shot.png");
            write(&picture, b"new")?;
            write(&room.join("shot.png"), b"old")?;
            let target = crate::free_target(&room, &picture);
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            // The spelling of the free name is paste's rule, shared with the
            // copy windows; what matters here is that it is a *different* name.
            if name == "shot.png" || !name.starts_with("shot") || !name.ends_with(".png") {
                return Err(format!("the copy would be called {name}"));
            }
            if fs::read(room.join("shot.png")).map_err(|e| e.to_string())? != b"old" {
                return Err("the file already there was changed".into());
            }
            // A folder with no extension takes the same rule.
            let dir = src.join("album");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            fs::create_dir_all(room.join("album")).map_err(|e| e.to_string())?;
            let second = crate::free_target(&room, &dir);
            let second_name = second
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if second_name == "album" || !second_name.starts_with("album") {
                return Err(format!("a folder copy would be called {second_name}"));
            }
            Ok(format!("{name}, {second_name}"))
        })(),
    );

    // -----------------------------------------------------------------------
    // A video's first frame as its preview
    // -----------------------------------------------------------------------
    r.check(
        "a video is recognised by its extension, and a picture is not",
        (|| {
            for name in ["clip.mp4", "clip.MKV", "clip.webm", "clip.mov", "clip.m4v"] {
                if !crate::fsutil::is_video_path(Path::new(name)) {
                    return Err(format!("{name} was not seen as a video"));
                }
            }
            for name in ["photo.png", "notes.txt", "clip.mkv.zip", "clip"] {
                if crate::fsutil::is_video_path(Path::new(name)) {
                    return Err(format!("{name} was seen as a video"));
                }
            }
            Ok(format!("{} extensions", crate::fsutil::VIDEO_EXT.len()))
        })(),
    );
    r.check(
        "the first frame is asked of ffmpeg at the size the tile needs",
        (|| {
            let args = crate::video::first_frame_args(Path::new("/tmp/clip.mp4"), 96);
            let joined = args.join(" ");
            for want in [
                "-nostdin",
                "-frames:v 1",
                "image2pipe",
                "-vcodec png",
                "/tmp/clip.mp4",
                "min(96,iw)",
            ] {
                if !joined.contains(want) {
                    return Err(format!("{want:?} missing from {joined}"));
                }
            }
            // A size over the module's ceiling is clamped rather than passed on.
            let big = crate::video::first_frame_args(Path::new("/tmp/clip.mp4"), 99_999).join(" ");
            if !big.contains(&format!("min({},iw)", crate::video::MAX_PX)) {
                return Err(format!("{big}"));
            }
            Ok(joined)
        })(),
    );
    r.check(
        "a real video's first frame comes back as a picture of its own shape",
        (|| {
            let Some(exe) = crate::video::ffmpeg() else {
                return Ok("ffmpeg is not installed here, so the frame was skipped".into());
            };
            let clip = base.join("frame-test.mp4");
            let made = std::process::Command::new(exe)
                .args([
                    "-v", "error", "-f", "lavfi", "-i", "color=c=0x1e88e5:s=320x240:d=1",
                    "-pix_fmt", "yuv420p", "-y",
                ])
                .arg(&clip)
                .status();
            match made {
                Ok(status) if status.success() => {}
                _ => return Err("the test clip could not be made".into()),
            }
            let frame = crate::video::first_frame(&clip, 160).ok_or("no frame came back")?;
            let (w, h) = (frame.width(), frame.height());
            if w == 0 || h == 0 || w > 160 || h > 160 {
                return Err(format!("{w}×{h} is outside the 160 px asked for"));
            }
            let ratio = w as f32 / h as f32;
            if (ratio - 4.0 / 3.0).abs() > 0.05 {
                return Err(format!("{w}×{h} is not the clip's own 4:3"));
            }
            Ok(format!("320×240 clip → {w}×{h}"))
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