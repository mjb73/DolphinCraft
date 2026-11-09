//! The kinds of document *Create New* can start, found from the applications
//! the desktop has registered.
//!
//! An application that keeps documents of its own says so three times over: its
//! `.desktop` entry names the type in `MimeType=` (`application/x-photocraft`,
//! beside the shared `image/png` and friends); its MIME package describes that
//! type (`PhotoCraft document`, `*.pcraft`) and may name the application's own
//! icon; and the description itself usually carries the application's name.
//! `apps::catalog` already reads the entries, so this module reads the MIME
//! packages the same way — the flatpak export first, then the user's own types,
//! then the system's — and joins the two on evidence the desktop wrote down.
//!
//! **Where the list does not come from: the file associations.** The desktop's
//! *default applications* answer which program *opens* a type, and that is a
//! different question from which one *makes* a new one. On this machine
//! `~/.config/mimeapps.list` assigns 77 types, 54 of them with a description and
//! an extension, and 27 of those are video containers assigned to VLC and 24 are
//! text formats assigned to the editor — a list of viewers, not of document
//! makers; PhotoCraft's own type is not assigned there at all. So the assignment
//! is deliberately not the catalog: the catalog is what each installed
//! application says is its own format, and a type is offered only when the
//! desktop's own records join it to an application unambiguously.
//!
//! **A type the build has no command for is still offered.** `create` makes the
//! empty file a file manager can always make and says so, because dropping the
//! row would be the hardcoding this list exists to avoid.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One registered document type: the application that owns it, the MIME type it
/// declares, the extension the desktop gives that type, and the words a row and
/// a file name use.
#[derive(Clone, Debug, PartialEq)]
pub struct DocKind {
    /// The application's name as its `.desktop` entry spells it — `PhotoCraft`.
    pub app: String,
    /// Its desktop id with `.desktop` taken off: `ai.storyteller.photocraft`.
    pub id: String,
    /// The type it registers: `application/x-photocraft`.
    pub mime: String,
    /// The type's first extension, without the dot: `pcraft`.
    pub ext: String,
    /// The MIME package's own description, title-cased: `PhotoCraft Document`.
    pub label: String,
}

impl DocKind {
    /// `New PhotoCraft Document.pcraft` — the name a new one starts with, before
    /// the in-place editor opens on it. The same name is the row's label with
    /// one word in front, so the two cannot drift apart.
    pub fn default_name(&self) -> String {
        format!("New {}.{}", self.label, self.ext)
    }
}

/// How a new document of a type is started.
pub enum Seed {
    /// Bytes this build can write itself — a format plain enough that a skeleton
    /// is a document.
    Text(&'static str),
    /// The application's own headless command. The file goes between the two
    /// halves: the words in front of it and the words after it. PhotoCraft's
    /// `run --new {} --out <file>` is one of the first and none of the second;
    /// FilmCraft's `--save-as <file> exec file.newProject …` is one of each.
    Cli {
        before: &'static [&'static str],
        after: &'static [&'static str],
    },
}

/// A VectorCraft document, at the size and shape its own New gives one: one
/// 1080 × 1080 artboard, no layers, and a `next_id` of 2 so the first thing
/// drawn gets 2. Read back by `vectorcraft-cli run --in … --export` with no
/// warnings, and by the app itself. A document's own `layers` may be empty —
/// that is a new file, not a broken one.
const VECTORCRAFT_SKELETON: &str = r#"{
  "format": "vectorcraft",
  "version": 4,
  "generator": "VectorCraft",
  "document": {
    "version": 3,
    "title": "Untitled",
    "units": "Pixels",
    "rulers": "artboard",
    "color_mode": "Rgb",
    "artboards": [
      {
        "id": 1,
        "name": "Artboard 1",
        "rect": { "x0": 0.0, "y0": 0.0, "x1": 1080.0, "y1": 1080.0 },
        "show_center_mark": false,
        "show_cross_hairs": false
      }
    ],
    "layers": [],
    "swatches": [],
    "swatch_groups": [],
    "graphic_styles": [],
    "symbols": [],
    "guides": [],
    "grid": { "spacing": 72.0, "subdivisions": 8 },
    "raster_effects_ppi": 72.0,
    "images": {},
    "next_id": 2,
    "metadata": {
      "author": "",
      "authorTitle": "",
      "description": "",
      "keywords": [],
      "rating": 0,
      "copyrightStatus": "unknown",
      "copyrightNotice": "",
      "copyrightUrl": "",
      "created": 0,
      "modified": 0
    },
    "last_view": {
      "name": "",
      "center": { "x": 540.0, "y": 540.0 },
      "zoom": 1.0,
      "rotation": 0.0
    }
  },
  "images": {}
}
"#;

/// The command that starts a new document of this type, per application.
///
/// A format this build can write itself needs no application: VectorCraft's
/// document is JSON, so the skeleton above is the whole document, and writing it
/// here is instant and works in a folder the sandbox was never given. Everything
/// else is asked of the application that owns the format, with the words its own
/// CLI documents for making one — the file is the only thing this build fills in,
/// so no format's internals are written here.
///
/// An application that is not in this table is still a registered document type
/// and still gets a row; `create` then makes an empty file of the right name and
/// says so, because a file manager can always do that much. Adding an application
/// is one arm.
fn seed_for(ext: &str, short: &str) -> Option<Seed> {
    if matches!(ext, "vectorcraft" | "drawcraft") {
        return Some(Seed::Text(VECTORCRAFT_SKELETON));
    }
    let before: &'static [&'static str] = match short {
        // `photocraft-cli run --new {} --out <file>`
        "photocraft" => &["run", "--new", "{}", "--out"],
        "vectorcraft" => &["run", "--export"],
        "deckcraft" | "soundcraft" => &["run", "--save"],
        // A word processor's page: the command has to run before the save.
        "designcraft" => &["run", "--cmd", "file.new", "--export"],
        "filmcraft" => &["--save-as"],
        _ => return None,
    };
    let after: &'static [&'static str] = match short {
        // `filmcraft-cli --save-as <file> exec file.newProject '{"name":"Untitled"}'`
        "filmcraft" => &["exec", "file.newProject", "{\"name\":\"Untitled\"}"],
        _ => &[],
    };
    Some(Seed::Cli { before, after })
}

/// Every document type the installed applications own, in label order. Read
/// once and kept: an application installed while the window is open shows up on
/// the next start, like the rest of the desktop's lists.
pub fn catalog() -> &'static Vec<DocKind> {
    static CATALOG: OnceLock<Vec<DocKind>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let packages = types();
        let mut out: Vec<DocKind> = Vec::new();
        for app in crate::apps::catalog() {
            let short = short_name(&app.id);
            for mime in &app.mimes {
                if out.iter().any(|d| &d.mime == mime) {
                    continue;
                }
                let Some(facts) = packages.get(mime) else {
                    continue;
                };
                if facts.comment.is_empty()
                    || !owns_type(mime, &short, &app.name, &app.icon, facts)
                {
                    continue;
                }
                let Some(ext) = first_extension(&facts.globs) else {
                    continue;
                };
                out.push(DocKind {
                    app: app.name.clone(),
                    id: app.id.trim_end_matches(".desktop").to_string(),
                    mime: mime.clone(),
                    ext,
                    label: title_case(&facts.comment),
                });
            }
        }
        out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
        // `DOLPHINCRAFT_DOCS_TRACE=1` prints the rows and where they came from,
        // which is how a machine that is not this one can be asked what its
        // registry offered without a debugger.
        if std::env::var_os("DOLPHINCRAFT_DOCS_TRACE").is_some() {
            for kind in &out {
                eprintln!(
                    "docs: {} <- {} {} ({})",
                    kind.mime, kind.app, kind.id, kind.ext
                );
            }
        }
        out
    })
}

/// The rows *New* offers, as indices into `catalog()`: the types the settings
/// have ticked, in the catalog's order. The settings keep MIME types rather than
/// indices, so a build that gains or loses a format cannot silently move a tick
/// to a different type.
pub fn enabled(config: &crate::config::Config) -> Vec<usize> {
    catalog()
        .iter()
        .enumerate()
        .filter(|(_, kind)| config.offers_new_doc(&kind.mime))
        .map(|(i, _)| i)
        .collect()
}

/// Make `target` a new document of this kind. `create_new` semantics: a name
/// already on disk is refused rather than overwritten.
///
/// `true` means a document the application opens; `false` means the type has no
/// known way to start one here, so the file is the empty one a file manager can
/// always make — the row exists because the type is registered, and the person is
/// told which of the two they got.
pub fn create(kind: &DocKind, target: &Path) -> std::io::Result<bool> {
    let short = short_name(&kind.id);
    match seed_for(&kind.ext, &short) {
        Some(Seed::Text(bytes)) => {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            file.write_all(bytes.as_bytes())?;
            Ok(true)
        }
        Some(Seed::Cli { before, after }) => {
            run_cli(kind, &short, before, after, target)?;
            Ok(true)
        }
        None => {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            Ok(false)
        }
    }
}

/// Ask the application to make the document, through the CLI that ships beside
/// it (`photocraft-cli` beside `photocraft`). The file has to exist and hold
/// something afterwards: an exit code of 0 that wrote nothing is still a failure,
/// and the CLI's own first line of complaint is the message the toast shows.
///
/// **Two ways to reach that CLI, because two ways to install one application.**
/// A package puts `<short>-cli` on the PATH; a flatpak runs it inside its own
/// sandbox (`flatpak run --command=<short>-cli <id>`). The PATH is tried first
/// and the store second, so the same row works on a machine that installed the
/// application with the distribution's package manager, and on one that got it
/// from the store. Nothing here names an application.
fn run_cli(
    kind: &DocKind,
    short: &str,
    before: &[&str],
    after: &[&str],
    target: &Path,
) -> std::io::Result<()> {
    let cli = format!("{}-cli", short);
    let mut plans: Vec<Vec<String>> = Vec::new();
    if crate::apps::which(&cli) {
        let mut args = vec![cli.clone()];
        args.extend(before.iter().map(|s| s.to_string()));
        args.push(target.to_string_lossy().to_string());
        args.extend(after.iter().map(|s| s.to_string()));
        plans.push(args);
    }
    let mut store = vec![
        "flatpak".to_string(),
        "run".to_string(),
        format!("--command={}", cli),
        kind.id.clone(),
    ];
    store.extend(before.iter().map(|s| s.to_string()));
    store.push(target.to_string_lossy().to_string());
    store.extend(after.iter().map(|s| s.to_string()));
    plans.push(store);

    let mut complaint: Option<String> = None;
    for args in plans {
        let mut command = std::process::Command::new(&args[0]);
        command
            .args(&args[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        // A document is named after its folder, so the command runs there: a
        // `flatpak run` whose working directory the sandbox cannot enter fails
        // before the application starts, and the file is what `--out` takes
        // either way.
        if let Some(dir) = target.parent() {
            command.current_dir(dir);
        }
        let out = match command.output() {
            Ok(out) => out,
            Err(e) => {
                complaint.get_or_insert(e.to_string());
                continue;
            }
        };
        let made = out.status.success()
            && std::fs::metadata(target).map(|m| m.len() > 0).unwrap_or(false);
        if std::env::var_os("DOLPHINCRAFT_DOCS_TRACE").is_some() {
            eprintln!(
                "docs: {:?} -> {} : {}",
                command,
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        if made {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let said = stderr.lines().map(str::trim).find(|l| !l.is_empty());
        let line = match said {
            Some(line) => line.to_string(),
            None => format!("{} exited with {}", kind.app, out.status),
        };
        complaint.get_or_insert(line);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        complaint.unwrap_or_else(|| format!("{} could not start a document", kind.app)),
    ))
}

/// The application's own short name: the last part of its desktop id. The
/// `.desktop` on the end is dropped first, so `ai.storyteller.photocraft.desktop`
/// answers `photocraft` and not `desktop`.
pub(crate) fn short_name(id: &str) -> String {
    let stem = id.strip_suffix(".desktop").unwrap_or(id);
    stem.rsplit('.').next().unwrap_or(stem).to_string()
}

/// Whether an installed application owns the type it declares.
///
/// The desktop's own records are the only evidence: the entry must list the
/// type (`MimeType=`), and the type must then read as that application's own
/// format. Three things say so, and any one is enough — there is no table of
/// application names here, and no rule that requires an application to be
/// called anything in particular:
///
/// * **the type is named after it** — `application/x-<short>`, where `<short>`
///   is the last part of its desktop id, optionally with a word after it
///   (`application/x-soundcraft-session`). This is how the Craft applications
///   declare their formats, and how Blender and Qt Designer do;
/// * **the package names the application's icon** — the type's `<icon name>`
///   is the entry's `Icon=`, so the format's own package says which application
///   it belongs to (`application/x-meld-comparison` → `org.gnome.Meld`);
/// * **the description names the application** — "FreeCAD document files",
///   "FreeFileSync Batch File". Only the start of the description counts, so a
///   sentence that merely mentions the program does not claim the type.
///
/// A `-template` type is a form to start from, not a document, and never counts;
/// `image/png` and the rest of the shared types are declared by many programs
/// and are no one's own.
pub(crate) fn owns_type(
    mime: &str,
    short: &str,
    app_name: &str,
    app_icon: &str,
    facts: &MimeFacts,
) -> bool {
    if mime.ends_with("-template") {
        return false;
    }
    if named_after_app(mime, short) {
        return true;
    }
    if !facts.icon.is_empty() && facts.icon.eq_ignore_ascii_case(app_icon.trim()) {
        return true;
    }
    let name = normalise(app_name);
    name.chars().count() >= 4 && normalise(&facts.comment).starts_with(&name)
}

/// Letters and digits only, lower-cased: "FreeCAD" and "FreeCAD Document"
/// compare equal on the parts that spell a name.
fn normalise(text: &str) -> String {
    text.chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Whether `mime` is the type the application whose short name this is declares
/// as its own. The applications declare their own type as `application/x-<short>`
/// (PhotoCraft, VectorCraft) and some add a word after it (SoundCraft's
/// `application/x-soundcraft-session`), so the prefix counts; a `-template` type
/// does not, because a template is not a document.
pub(crate) fn named_after_app(mime: &str, short: &str) -> bool {
    let Some(rest) = mime.strip_prefix("application/x-") else {
        return false;
    };
    if rest == short {
        return true;
    }
    match rest.strip_prefix(short) {
        Some(tail) => tail.starts_with('-') && !tail.ends_with("-template"),
        None => false,
    }
}

/// The first plain extension in a type's glob list — `*.pcraft` answers `pcraft`.
/// A glob with a wildcard anywhere but the front names a family, not a name, so
/// it is passed over; the applications list the canonical one first.
pub(crate) fn first_extension(globs: &[String]) -> Option<String> {
    for glob in globs {
        let Some(rest) = glob.strip_prefix("*.") else {
            continue;
        };
        if rest.is_empty()
            || rest.contains(|c: char| {
                c.is_whitespace() || matches!(c, '*' | '?' | '[' | ']' | '/')
            })
        {
            continue;
        }
        return Some(rest.to_ascii_lowercase());
    }
    None
}

/// `PhotoCraft document` → `PhotoCraft Document`. A word keeps the capitals it
/// arrived with; only the first letter of each word is raised, because the MIME
/// packages write their comments as sentences and the rows want names.
pub(crate) fn title_case(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// What a MIME package says about one type: the words a row uses, the glob its
/// file name comes from, and — when the package bothers to say — the icon its
/// application carries. The icon is the package and the `.desktop` entry
/// agreeing about who owns the type, which is worth more than a name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MimeFacts {
    pub comment: String,
    pub globs: Vec<String>,
    pub icon: String,
}

/// Every type one package describes, in the order the file lists them.
pub(crate) fn parse_mime_types(text: &str) -> Vec<(String, MimeFacts)> {
    let mut out: Vec<(String, MimeFacts)> = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("<mime-type") {
        let start = from + i;
        let Some(head_end) = text[start..].find('>') else { break };
        let head = &text[start..start + head_end];
        let Some(at) = head.find("type=\"") else {
            from = start + head_end + 1;
            continue;
        };
        let value = start + at + "type=\"".len();
        let Some(len) = text[value..].find('"') else { break };
        let mime = text[value..value + len].to_string();
        let Some(end) = text[start..].find("</mime-type>") else { break };
        if let Some(facts) = facts_in(&text[start..start + end]) {
            out.push((mime, facts));
        }
        from = start + end + "</mime-type>".len();
    }
    out
}

/// One type out of a package: its `comment`, its glob patterns and its icon.
/// Localised `comment` rows carry attributes and are skipped, so the plain one
/// wins.
pub(crate) fn parse_mime_package(text: &str, mime: &str) -> Option<MimeFacts> {
    parse_mime_types(text)
        .into_iter()
        .find(|(t, _)| t == mime)
        .map(|(_, facts)| facts)
}

fn facts_in(block: &str) -> Option<MimeFacts> {
    let comment = block
        .find("<comment>")
        .and_then(|i| {
            let after = &block[i + "<comment>".len()..];
            after.find("</comment>").map(|j| after[..j].trim().to_string())
        })
        .unwrap_or_default();
    let icon = block
        .find("<icon name=\"")
        .and_then(|i| {
            let after = &block[i + "<icon name=\"".len()..];
            after.find('"').map(|j| after[..j].to_string())
        })
        .unwrap_or_default();
    let mut globs = Vec::new();
    let mut from = 0;
    while let Some(i) = block[from..].find("pattern=\"") {
        let s = from + i + "pattern=\"".len();
        let Some(j) = block[s..].find('"') else { break };
        globs.push(block[s..s + j].to_string());
        from = s + j + 1;
    }
    if comment.is_empty() && globs.is_empty() {
        return None;
    }
    Some(MimeFacts {
        comment,
        globs,
        icon,
    })
}

/// Where the desktop keeps its MIME packages: the user's own first, then the
/// flatpak exports (where an application's declarations land without being
/// installed system-wide), then every data directory and the system-wide
/// flatpak store.
fn mime_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = crate::fsutil::home_dir();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        dirs.push(xdg.join("mime/packages"));
        dirs.push(xdg.join("flatpak/exports/share/mime/packages"));
    } else {
        dirs.push(home.join(".local/share/mime/packages"));
        dirs.push(home.join(".local/share/flatpak/exports/share/mime/packages"));
    }
    let extra = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for part in extra.split(':').filter(|p| !p.is_empty()) {
        dirs.push(PathBuf::from(part).join("mime/packages"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/mime/packages"));
    dirs
}

/// Every type every package above describes, read once and kept — the same
/// whole-registry read the file associations do, so a type an application's
/// package never describes cannot be offered. The first file that describes a
/// type wins, which is why the directories are ordered the way they are.
fn types() -> &'static std::collections::HashMap<String, MimeFacts> {
    static TYPES: OnceLock<std::collections::HashMap<String, MimeFacts>> = OnceLock::new();
    TYPES.get_or_init(|| {
        let mut out: std::collections::HashMap<String, MimeFacts> =
            std::collections::HashMap::new();
        for dir in mime_dirs() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "xml").unwrap_or(false))
                .collect();
            files.sort();
            for path in files {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (mime, facts) in parse_mime_types(&text) {
                    out.entry(mime).or_insert(facts);
                }
            }
        }
        out
    })
}

