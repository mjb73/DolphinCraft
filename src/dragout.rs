//! Added feature: *drag a picture out of the window, into another program*.
//!
//! The person's words: "It should be possible to drag images from dolphincraft
//! to other apps."
//!
//! Nothing in the toolkit can do this. winit has no drag *source* on either
//! backend — its X11 side implements XDND only as a destination, and its Wayland
//! side has no data-device code at all — and egui's own drag-and-drop payloads
//! never leave the program. So the drag is spoken directly to the X server here,
//! in the XDND protocol the desktop already uses for this:
//!
//! 1. the drag takes the `XdndSelection` selection with its own small X window,
//!    which is also the window the target answers on;
//! 2. it follows the pointer and offers `text/uri-list`, `UTF8_STRING` and
//!    `text/plain` — the three ways a program asks for files — to whatever
//!    window under the cursor says it is a drag destination (`XdndAware`);
//! 3. on release it sends `XdndDrop` and serves the list of `file://` addresses.
//!
//! Two consequences are worth knowing:
//!
//! * The drag is **not grabbed**: the window being dragged *from* already holds
//!   the pointer for the duration of the button press, and a second X connection
//!   cannot take a grab another client holds. The pointer is polled instead
//!   (`QueryPointer`), which is enough to follow the drag and to notice the
//!   release.
//! * It is X11 only, so a Wayland session needs the window on XWayland — which
//!   is where the window already goes for a capture, and which KWin bridges to
//!   Wayland-native programs in both directions. `main` asks for the X11
//!   backend on Linux for exactly this reason.
//!
//! The XDND protocol itself: <https://www.freedesktop.org/wiki/Specifications/XDND/>

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// How far the pointer has to travel, with the button held down, before a press
/// on an item is a drag rather than a click.
pub const THRESHOLD: f32 = 8.0;

/// Whether a press that started at `from` has reached `to` — a drag has begun.
pub fn far_enough(from: egui::Pos2, to: egui::Pos2) -> bool {
    to.distance(from) >= THRESHOLD
}

/// The `text/uri-list` body for these files: one `file://` address per line,
/// each line ending `\r\n`, which is what the format asks for.
pub fn uri_list(files: &[PathBuf]) -> String {
    let mut out = String::new();
    for file in files {
        out.push_str("file://");
        out.push_str(&escape_path(file));
        out.push_str("\r\n");
    }
    out
}

/// The `text/plain` body: the paths themselves, one per line, for the odd
/// program that asks for text rather than a list of files.
pub fn plain_list(files: &[PathBuf]) -> String {
    let mut out = String::new();
    for file in files {
        out.push_str(&file.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Percent-escape a path the way a `file://` address needs it: everything
/// outside the unreserved set and the separator becomes `%XX`. A space, a `#`,
/// a `?` or a non-ASCII letter in a file name would otherwise break the address.
pub fn escape_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len() + 8);
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// What came of one drag, for the sentence afterwards.
pub struct Outcome {
    pub files: usize,
    /// How many times a receiving program asked for the list (normally once).
    pub delivered: usize,
    /// A window accepted the drag well enough to be sent the drop.
    pub took_drop: bool,
    /// The receiving program said the transfer went through (XDND v5).
    pub acknowledged: bool,
    pub reason: Option<String>,
}

impl Outcome {
    /// The sentence the toast shows when the drag ends. `None` means the gesture
    /// was a stray press that ended over nothing — a click, in other words, and
    /// not worth a word.
    pub fn message(&self) -> Option<String> {
        if let Some(why) = &self.reason {
            return Some(format!("The drag did not finish — {why}"));
        }
        let items = if self.files == 1 {
            "the item".to_string()
        } else {
            format!("the {} items", self.files)
        };
        if self.delivered > 0 {
            return Some(format!("Handed {items} to the other program."));
        }
        if self.took_drop {
            return Some("The other program took the drop but asked for nothing.".to_string());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// The X11 side
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod x11 {
    use super::*;

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, CreateWindowAux,
        EventMask, PropMode, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
        SELECTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// Buttons 1–3 held, as `QueryPointer` reports them.
    const HELD: u16 = (1 << 8) | (1 << 9) | (1 << 10);

    /// A drag is a conversation between two programs, so when one of them looks
    /// dead the only way to tell what happened is to write down what was said.
    /// `DOLPHINCRAFT_DRAG_TRACE=1` turns that on; it is silent otherwise.
    macro_rules! trace {
        ($($arg:tt)*) => {
            if std::env::var_os("DOLPHINCRAFT_DRAG_TRACE").is_some() {
                eprintln!("drag trace: {}", format!($($arg)*));
            }
        };
    }

    /// The most a single property write carries before the transfer switches to
    /// `INCR` chunking (the X server refuses requests above about 256 KB).
    const INLINE_LIMIT: usize = 96 * 1024;

    /// One `INCR` transfer in flight: the program that asked, the property it
    /// reads from, and what is left to send.
    struct Chunked {
        requestor: Window,
        property: u32,
        type_: u32,
        data: Vec<u8>,
        sent: usize,
    }

    /// The window a drag is currently over, and whether it has said it will
    /// take what is being carried.
    struct Aim {
        win: Window,
        accepts: bool,
    }

    struct Atoms {
        selection: u32,
        enter: u32,
        position: u32,
        status: u32,
        leave: u32,
        drop: u32,
        finished: u32,
        aware: u32,
        action_copy: u32,
        uri_list: u32,
        utf8: u32,
        text_plain: u32,
        targets: u32,
        incr: u32,
        clock: u32,
    }

    impl Atoms {
        fn intern(conn: &RustConnection) -> Result<Self, String> {
            let name = |text: &str| -> Result<u32, String> {
                conn.intern_atom(false, text.as_bytes())
                    .map_err(|e| format!("X would not intern {text}: {e}"))?
                    .reply()
                    .map(|r| r.atom)
                    .map_err(|e| format!("X would not intern {text}: {e}"))
            };
            Ok(Self {
                selection: name("XdndSelection")?,
                enter: name("XdndEnter")?,
                position: name("XdndPosition")?,
                status: name("XdndStatus")?,
                leave: name("XdndLeave")?,
                drop: name("XdndDrop")?,
                finished: name("XdndFinished")?,
                aware: name("XdndAware")?,
                action_copy: name("XdndActionCopy")?,
                uri_list: name("text/uri-list")?,
                utf8: name("UTF8_STRING")?,
                text_plain: name("text/plain")?,
                targets: name("TARGETS")?,
                incr: name("INCR")?,
                clock: name("DolphinCraftDragClock")?,
            })
        }
    }

    /// A drag in progress, owned by the window that started it.
    pub struct Drag {
        pub files: usize,
        rx: Receiver<Outcome>,
        started: Instant,
    }

    impl Drag {
        /// Take hold of the pointer's current position and start following it.
        /// Returns at once; the drag itself runs on its own thread, because it
        /// has to answer the receiving program while the window keeps drawing.
        pub fn start(paths: Vec<PathBuf>) -> Result<Self, String> {
            if paths.is_empty() {
                return Err("Nothing was marked to drag.".into());
            }
            if std::env::var_os("DISPLAY").is_none() && !cfg!(windows) {
                return Err("this window is not on an X display, so it cannot hand a drag over".into());
            }
            let files = paths.len();
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let outcome = follow(&paths);
                let _ = tx.send(outcome);
            });
            Ok(Self {
                files,
                rx,
                started: Instant::now(),
            })
        }

        pub fn elapsed(&self) -> Duration {
            self.started.elapsed()
        }

        /// `Some` once the drag has ended.
        pub fn poll(&mut self) -> Option<Outcome> {
            match self.rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Outcome {
                    files: self.files,
                    delivered: 0,
                    took_drop: false,
                    acknowledged: false,
                    reason: Some("the drag stopped before it ended".into()),
                }),
            }
        }
    }

    fn follow(paths: &[PathBuf]) -> Outcome {
        match drag_out(paths) {
            Ok(mut outcome) => {
                outcome.files = paths.len();
                outcome
            }
            Err(reason) => Outcome {
                files: paths.len(),
                delivered: 0,
                took_drop: false,
                acknowledged: false,
                reason: Some(reason),
            },
        }
    }

    fn drag_out(paths: &[PathBuf]) -> Result<Outcome, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("no X display to drag onto ({e})"))?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|s| s.root)
            .ok_or_else(|| "the X display has no screen".to_string())?;
        let atoms = Atoms::intern(&conn)?;

        let win: Window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            win,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .override_redirect(1)
                .event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| format!("the drag's own window could not be made ({e})"))?;
        // The selection is how a receiving program and, on a Wayland session,
        // the compositor's own bridge learn that an X drag is under way.
        conn.set_selection_owner(win, atoms.selection, x11rb::CURRENT_TIME)
            .map_err(|e| format!("the drag could not take the X selection ({e})"))?;
        conn.flush().map_err(|e| e.to_string())?;
        let time = server_time(&conn, win, atoms.clock).unwrap_or(x11rb::CURRENT_TIME);

        let mut served = 0usize;
        let mut chunked: Vec<Chunked> = Vec::new();
        let mut aim: Option<Aim> = None;
        let mut last_target: Window = x11rb::NONE;
        let mut finished: Option<bool> = None;
        let mut last_sent: Option<(i16, i16)> = None;
        let mut last_position_at = Instant::now() - Duration::from_secs(1);
        let deadline = Instant::now() + Duration::from_secs(180);

        // Follow the pointer until the button that began the drag comes up.
        loop {
            if Instant::now() > deadline {
                let _ = let_go(&conn, win, &atoms);
                return Err("the drag was held down for too long".into());
            }
            pump(
                &conn,
                &atoms,
                win,
                paths,
                &mut served,
                &mut chunked,
                &mut aim,
                &mut last_target,
                &mut finished,
            )?;
            let pointer = conn
                .query_pointer(root)
                .map_err(|e| format!("the pointer could not be asked for ({e})"))?
                .reply()
                .map_err(|e| format!("the pointer could not be asked for ({e})"))?;
            if u16::from(pointer.mask) & HELD == 0 {
                trace!("the pointer reports no button held ({})", u16::from(pointer.mask));
                break;
            }
            let under = deepest(&conn, root, pointer.root_x, pointer.root_y)?;
            let found = destination(&conn, under, root, atoms.aware)?;
            match (&aim, found) {
                (Some(t), Some((w, _))) if t.win == w => {}
                (Some(t), _) => {
                    trace!("leaving window {} (now over {:?})", t.win, found);
                    send_leave(&conn, &atoms, win, t.win)?;
                    aim = None;
                }
                (None, _) => {}
            }
            if let (None, Some((w, version))) = (&aim, found) {
                trace!("entering window {w} (protocol {version})");
                send_enter(&conn, &atoms, win, w, version)?;
                last_target = w;
                aim = Some(Aim {
                    win: w,
                    accepts: false,
                });
                last_sent = None;
            }
            if let Some(target) = aim.as_ref() {
                let moved = last_sent
                    .map(|(x, y)| (x - pointer.root_x).abs() > 1 || (y - pointer.root_y).abs() > 1)
                    .unwrap_or(true);
                let spaced = last_position_at.elapsed() >= Duration::from_millis(20);
                if moved && spaced {
                    send_position(
                        &conn,
                        &atoms,
                        win,
                        target.win,
                        pointer.root_x,
                        pointer.root_y,
                        time,
                    )?;
                    last_sent = Some((pointer.root_x, pointer.root_y));
                    last_position_at = Instant::now();
                }
            }
            std::thread::sleep(Duration::from_millis(12));
        }

        // The button came up: hand the drop over, or back out of the window it
        // was over if that program was not taking it.
        trace!("the drag ended over {:?}", aim.as_ref().map(|t| (t.win, t.accepts)));
        let dropped = match aim.as_ref() {
            Some(target) if target.accepts => {
                send_drop(&conn, &atoms, win, target.win, time)?;
                true
            }
            Some(target) => {
                send_leave(&conn, &atoms, win, target.win)?;
                false
            }
            None => false,
        };

        // The receiving program now asks for the list of addresses. Keep
        // answering until it says it is finished (`XdndFinished`, protocol 5),
        // or until it has gone quiet, which is all the older protocol gives.
        if dropped {
            let wait_until = Instant::now() + Duration::from_secs(20);
            let mut quiet_for = Duration::from_millis(0);
            let mut served_before = served;
            loop {
                pump(
                    &conn,
                    &atoms,
                    win,
                    paths,
                    &mut served,
                    &mut chunked,
                    &mut aim,
                    &mut last_target,
                    &mut finished,
                )?;
                if served != served_before {
                    served_before = served;
                    quiet_for = Duration::from_millis(0);
                } else {
                    quiet_for += Duration::from_millis(10);
                }
                if finished.is_some() {
                    break;
                }
                if Instant::now() > wait_until {
                    break;
                }
                if served > 0 && quiet_for >= Duration::from_millis(1500) {
                    break;
                }
                // A drop into a program that never asks for anything at all
                // still has to end.
                if served == 0 && quiet_for >= Duration::from_secs(8) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let _ = let_go(&conn, win, &atoms);
        Ok(Outcome {
            files: paths.len(),
            delivered: served,
            took_drop: dropped,
            acknowledged: finished.unwrap_or(dropped),
            reason: None,
        })
    }

    /// Read the messages a receiving program sends, and answer the ones that
    /// ask for the list. Returns nothing: everything learned is written back
    /// through the arguments.
    #[allow(clippy::too_many_arguments)]
    fn pump(
        conn: &RustConnection,
        atoms: &Atoms,
        _win: Window,
        paths: &[PathBuf],
        served: &mut usize,
        chunked: &mut Vec<Chunked>,
        aim: &mut Option<Aim>,
        last_target: &mut Window,
        finished: &mut Option<bool>,
    ) -> Result<(), String> {
        while let Some(event) = conn.poll_for_event().map_err(|e| e.to_string())? {
            match event {
                Event::ClientMessage(message) => {
                    let data = message.data.as_data32();
                    // `XdndStatus` and `XdndFinished` name the *target* window in
                    // data[0] — not the source — so the answer is matched against
                    // the window the drag is over.
                    if message.type_ == atoms.status {
                        if let Some(target) = aim.as_mut() {
                            if data[0] == target.win {
                                target.accepts = data[1] & 1 != 0;
                                trace!("window {} status accept={}", target.win, target.accepts);
                            }
                        }
                    } else if message.type_ == atoms.finished && data[0] == *last_target {
                        *finished = Some(data[1] & 1 != 0);
                        trace!("window {} finished ok={}", *last_target, data[1] & 1 != 0);
                    }
                }
                Event::SelectionRequest(request) => {
                    if request.selection == atoms.selection {
                        serve(conn, atoms, paths, request, served, chunked)?;
                    }
                }
                Event::PropertyNotify(change) => {
                    if change.state == x11rb::protocol::xproto::Property::DELETE {
                        write_chunk(conn, atoms, change.window, change.atom, chunked)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Answer one `SelectionRequest`: write the list into the property the
    /// asking program named, then tell it the property is there.
    fn serve(
        conn: &RustConnection,
        atoms: &Atoms,
        paths: &[PathBuf],
        request: SelectionRequestEvent,
        served: &mut usize,
        chunked: &mut Vec<Chunked>,
    ) -> Result<(), String> {
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };
        trace!(
            "asked for atom {} (property {}) by window {}",
            request.target,
            request.property,
            request.requestor
        );
        let mut answered = true;
        if request.target == atoms.targets {
            let list = [atoms.targets, atoms.uri_list, atoms.utf8, atoms.text_plain];
            conn.change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                AtomEnum::ATOM,
                &list,
            )
            .map_err(|e| format!("the offered types could not be written ({e})"))?;
        } else if request.target == atoms.uri_list || request.target == atoms.utf8
            || request.target == atoms.text_plain
        {
            let type_ = if request.target == atoms.uri_list {
                atoms.uri_list
            } else {
                atoms.utf8
            };
            let body = if request.target == atoms.uri_list {
                uri_list(paths)
            } else {
                plain_list(paths)
            };
            if body.len() > INLINE_LIMIT {
                // Too much for one property write: say how much is coming and
                // send it in chunks as the property is emptied.
                let total = body.len() as u32;
                conn.change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    atoms.incr,
                    &[total],
                )
                .map_err(|e| format!("the size of the list could not be written ({e})"))?;
                conn.change_window_attributes(
                    request.requestor,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .map_err(|e| format!("the transfer could not be watched ({e})"))?;
                trace!("the list is {} bytes, so it is sent in chunks", body.len());
                chunked.push(Chunked {
                    requestor: request.requestor,
                    property,
                    type_,
                    data: body.into_bytes(),
                    sent: 0,
                });
            } else {
                conn.change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    type_,
                    body.as_bytes(),
                )
                .map_err(|e| format!("the list could not be written ({e})"))?;
            }
        } else {
            answered = false;
        }
        conn.send_event(
            false,
            request.requestor,
            EventMask::NO_EVENT,
            SelectionNotifyEvent {
                response_type: SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: request.time,
                requestor: request.requestor,
                selection: request.selection,
                target: request.target,
                property: if answered { property } else { x11rb::NONE },
            },
        )
        .map_err(|e| format!("the transfer could not be announced ({e})"))?;
        conn.flush().map_err(|e| e.to_string())?;
        if answered {
            *served += 1;
        }
        Ok(())
    }

    /// One step of an `INCR` transfer: the receiving program emptied the
    /// property, so write the next piece — or, once every byte has been taken,
    /// the zero-length property that says the list is complete.
    ///
    /// The end marker waits for that last delete. Writing it straight after the
    /// final piece would land in the same property before the receiver had read
    /// it, and the tail of the list would silently vanish.
    fn write_chunk(
        conn: &RustConnection,
        atoms: &Atoms,
        window: Window,
        property: u32,
        chunked: &mut Vec<Chunked>,
    ) -> Result<(), String> {
        let Some(index) = chunked
            .iter()
            .position(|c| c.requestor == window && c.property == property)
        else {
            return Ok(());
        };
        if chunked[index].sent >= chunked[index].data.len() {
            let type_ = chunked[index].type_;
            conn.change_property8(PropMode::REPLACE, window, property, type_, &[])
                .map_err(|e| format!("the end of the list could not be written ({e})"))?;
            chunked.remove(index);
            conn.flush().map_err(|e| e.to_string())?;
            trace!("the last part of the list was taken; the transfer is closed");
            return Ok(());
        }
        let piece = {
            let transfer = &chunked[index];
            let end = (transfer.sent + 64 * 1024).min(transfer.data.len());
            transfer.data[transfer.sent..end].to_vec()
        };
        let type_ = chunked[index].type_;
        chunked[index].sent += piece.len();
        let done = chunked[index].sent >= chunked[index].data.len();
        trace!("writing {} bytes of the list (last={done})", piece.len());
        conn.change_property8(PropMode::REPLACE, window, property, type_, &piece)
            .map_err(|e| format!("the next part of the list could not be written ({e})"))?;
        conn.flush().map_err(|e| e.to_string())?;
        let _ = atoms;
        Ok(())
    }

    /// The deepest window under the pointer, starting from the root.
    fn deepest(conn: &RustConnection, from: Window, x: i16, y: i16) -> Result<Window, String> {
        let mut window = from;
        let _ = (x, y);
        for _ in 0..64 {
            let reply = conn
                .query_pointer(window)
                .map_err(|e| format!("the window under the pointer could not be found ({e})"))?
                .reply()
                .map_err(|e| format!("the window under the pointer could not be found ({e})"))?;
            if reply.child == x11rb::NONE || !reply.same_screen {
                return Ok(window);
            }
            window = reply.child;
        }
        Ok(window)
    }

    /// Walk up from the window under the pointer to the nearest one that says it
    /// takes drags, and read the protocol version it speaks.
    fn destination(
        conn: &RustConnection,
        from: Window,
        root: Window,
        aware: u32,
    ) -> Result<Option<(Window, u32)>, String> {
        let mut window = from;
        for _ in 0..64 {
            if let Some(version) = aware_version(conn, window, aware)? {
                return Ok(Some((window, version)));
            }
            if window == root || window == x11rb::NONE {
                return Ok(None);
            }
            let tree = conn
                .query_tree(window)
                .map_err(|e| format!("the window above could not be found ({e})"))?
                .reply()
                .map_err(|e| format!("the window above could not be found ({e})"))?;
            if tree.parent == x11rb::NONE || tree.parent == window {
                return Ok(None);
            }
            window = tree.parent;
        }
        Ok(None)
    }

    fn aware_version(
        conn: &RustConnection,
        window: Window,
        aware: u32,
    ) -> Result<Option<u32>, String> {
        let reply = conn
            .get_property(false, window, aware, AtomEnum::ANY, 0, 1)
            .map_err(|e| format!("a drag destination could not be asked ({e})"))?
            .reply()
            .map_err(|e| format!("a drag destination could not be asked ({e})"))?;
        if reply.format == 0 || reply.value.is_empty() {
            return Ok(None);
        }
        if reply.format == 32 && reply.value.len() >= 4 {
            let version = u32::from_ne_bytes([
                reply.value[0],
                reply.value[1],
                reply.value[2],
                reply.value[3],
            ]);
            return Ok(Some(version.max(1)));
        }
        // A window that answers but does not spell out a version is a version 1
        // destination, which is still worth offering the list to.
        Ok(Some(1))
    }

    fn send_client(
        conn: &RustConnection,
        target: Window,
        type_: u32,
        data: [u32; 5],
    ) -> Result<(), String> {
        conn.send_event(
            false,
            target,
            EventMask::NO_EVENT,
            ClientMessageEvent::new(32, target, type_, x11rb::protocol::xproto::ClientMessageData::from(data)),
        )
        .map_err(|e| format!("a drag message could not be sent ({e})"))?;
        conn.flush().map_err(|e| e.to_string())
    }

    fn send_enter(
        conn: &RustConnection,
        atoms: &Atoms,
        source: Window,
        target: Window,
        version: u32,
    ) -> Result<(), String> {
        let ours = version.min(5);
        let data = [
            source,
            ours << 24,
            atoms.uri_list,
            atoms.utf8,
            atoms.text_plain,
        ];
        send_client(conn, target, atoms.enter, data)
    }

    fn send_position(
        conn: &RustConnection,
        atoms: &Atoms,
        source: Window,
        target: Window,
        x: i16,
        y: i16,
        time: u32,
    ) -> Result<(), String> {
        let packed = ((x as u32) << 16) | (y as u32 & 0xFFFF);
        let data = [source, 0, packed, time, atoms.action_copy];
        send_client(conn, target, atoms.position, data)
    }

    fn send_leave(
        conn: &RustConnection,
        atoms: &Atoms,
        source: Window,
        target: Window,
    ) -> Result<(), String> {
        send_client(conn, target, atoms.leave, [source, 0, 0, 0, 0])
    }

    fn send_drop(
        conn: &RustConnection,
        atoms: &Atoms,
        source: Window,
        target: Window,
        time: u32,
    ) -> Result<(), String> {
        send_client(conn, target, atoms.drop, [source, 0, time, 0, 0])
    }

    /// Give the selection back and take the drag's window away.
    fn let_go(conn: &RustConnection, win: Window, atoms: &Atoms) -> Result<(), String> {
        conn.set_selection_owner(x11rb::NONE, atoms.selection, x11rb::CURRENT_TIME)
            .map_err(|e| e.to_string())?;
        conn.destroy_window(win).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())
    }

    /// A real server timestamp. Some drag destinations will not act on a message
    /// stamped `CurrentTime`, and this is the cheapest way to ask the server for
    /// the time it is actually at: change a property and read the event's stamp.
    fn server_time(conn: &RustConnection, win: Window, property: u32) -> Option<u32> {
        conn.change_property8(PropMode::REPLACE, win, property, AtomEnum::STRING, b"now")
            .ok()?;
        conn.flush().ok()?;
        let limit = Instant::now() + Duration::from_millis(400);
        while Instant::now() < limit {
            match conn.poll_for_event().ok()? {
                Some(Event::PropertyNotify(change)) if change.atom == property => {
                    return Some(change.time)
                }
                Some(_) => continue,
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        None
    }
}

#[cfg(target_os = "linux")]
pub use x11::Drag;

/// Other desktops: the X11 protocol here has nothing to talk to, so the same
/// shape is offered and every drag says why it cannot happen.
#[cfg(not(target_os = "linux"))]
mod elsewhere {
    use super::*;

    pub struct Drag {
        pub files: usize,
        started: Instant,
        said: bool,
    }

    impl Drag {
        pub fn start(paths: Vec<PathBuf>) -> Result<Self, String> {
            Ok(Self {
                files: paths.len(),
                started: Instant::now(),
                said: false,
            })
        }

        pub fn elapsed(&self) -> Duration {
            self.started.elapsed()
        }

        pub fn poll(&mut self) -> Option<Outcome> {
            if self.said {
                return None;
            }
            self.said = true;
            Some(Outcome {
                files: self.files,
                delivered: 0,
                took_drop: false,
                acknowledged: false,
                reason: Some("dragging out of the window is wired for the Linux desktop only".into()),
            })
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub use elsewhere::Drag;
