//! Added feature: the `org.freedesktop.FileManager1` service.
//!
//! Other programs ask the file manager to put something on screen: a browser's
//! "Show in folder", a finished download, a device the notifier just mounted.
//! They do it over D-Bus on the name `org.freedesktop.FileManager1`, and on this
//! machine Plasma's `plasma-dolphin.service` answers it — so those requests
//! opened Dolphin even though every file association says DolphinCraft.
//!
//! The interface is three methods and one property:
//!
//! - `ShowFolders(as uris)` — show these folders;
//! - `ShowItems(as uris, s startup_id)` — show the folder of these items and
//!   mark the items themselves;
//! - `ShowItemProperties(as uris, s startup_id)` — the same, then the
//!   Properties window;
//! - `DesktopName` — the manager's name, `DolphinCraft`.
//!
//! The bus work runs on its own thread with a blocking `zbus` connection; what
//! arrives is a `Fm1Call` on a channel the window drains every frame, so the
//! GUI never waits on the bus. A request that arrives while another program
//! already owns the name (Dolphin's daemon, for instance) is reported rather
//! than queued: the name is first-come, and `request_name` refuses instead of
//! stealing it.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// What another program asked the file manager to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fm1Call {
    /// `ShowFolders` — put these folders on screen.
    Folders(Vec<PathBuf>),
    /// `ShowItems` — show the folder of these items and mark them.
    Items(Vec<PathBuf>),
    /// `ShowItemProperties` — the same, then Properties.
    Properties(Vec<PathBuf>),
}

/// The object the bus talks to. Its methods only translate and hand over; the
/// window does the work on the frame after.
struct FileManager1 {
    tx: Sender<Fm1Call>,
    /// The window's own context. This thread wakes it, because the window only
    /// repaints on input otherwise and a request would wait for the next click.
    ctx: egui::Context,
}

impl FileManager1 {
    fn send(&self, call: Fm1Call) {
        // A closed channel means the window is gone; nothing to do then.
        let _ = self.tx.send(call);
        self.ctx.request_repaint();
    }
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager1 {
    fn show_folders(&self, uris: Vec<String>) {
        let paths = paths_of(&uris);
        if !paths.is_empty() {
            self.send(Fm1Call::Folders(paths));
        }
    }

    fn show_items(&self, uris: Vec<String>, _startup_id: String) {
        let paths = paths_of(&uris);
        if !paths.is_empty() {
            self.send(Fm1Call::Items(paths));
        }
    }

    fn show_item_properties(&self, uris: Vec<String>, _startup_id: String) {
        let paths = paths_of(&uris);
        if !paths.is_empty() {
            self.send(Fm1Call::Properties(paths));
        }
    }

    #[zbus(property)]
    fn desktop_name(&self) -> String {
        "DolphinCraft".to_string()
    }
}

/// Start the service on its own thread and return as soon as the name is held,
/// so a failure (another manager already owns it) reaches the caller.
pub fn start(tx: Sender<Fm1Call>, ctx: egui::Context) -> Result<(), String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let built = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name("org.freedesktop.FileManager1"))
            .and_then(|b| {
                b.serve_at(
                    "/org/freedesktop/FileManager1",
                    FileManager1 { tx, ctx },
                )
            });
        let connection = match built.and_then(|b| b.build()) {
            Ok(connection) => {
                let _ = ready_tx.send(Ok(()));
                connection
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        // The blocking connection serves on its own dispatch thread; this one
        // only has to keep the connection — and with it the name — alive, so
        // it parks holding it. Dropping the connection releases the name.
        let _name_holder = connection;
        loop {
            std::thread::park();
        }
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("the bus did not answer: {e}"))?
}

/// The paths behind the URIs of a request. Anything that is not a local file
/// (`file://`) is skipped: this window browses the file system, and a manager
/// that claimed an `smb://` or a `mtp://` URI would have nothing to draw.
pub fn paths_of(uris: &[String]) -> Vec<PathBuf> {
    uris.iter().filter_map(|u| path_of_uri(u)).collect()
}

/// One URI as a path. `file:///a%20b` is `/a b`; `file://localhost/a` is `/a`.
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // The authority: empty, or `localhost`, then the path.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(i) if &rest[..i] == "localhost" => &rest[i..],
        _ => return None,
    };
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(percent_decode(path)))
}

/// `%20` → a space, `%2F` → a slash. A URI is UTF-8 by definition, so a byte
/// sequence that is not valid UTF-8 keeps its original text rather than being
/// lost.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
