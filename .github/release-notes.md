DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.7

* **Create New offers the document types this computer actually has** — no list of
  applications in the code. Every entry is read from the desktop's own records
  (`/usr/share/applications`, `~/.local/share`, the Flatpak exports), and a type is
  offered only when three pieces of standard evidence say the application owns it:
  a MIME type named after it (`application/x-photocraft`), a MIME package whose icon
  matches its `Icon=`, or a description that opens with its name ("FreeCAD document
  files"). The document types are a `New Document ▸` submenu of *New*, not fifteen
  rows in the menu, and *Document Types (n/N) ▸* ticks which ones you want. On this
  machine it finds fifteen — six studio formats, Blender, Qt Designer, FLUID,
  FontForge, FreeCAD, FreeFileSync, RealTimeSync and Meld.
* **A created document is a real document.** The row runs the application's own CLI
  (`photocraft-cli run --new …`, `flatpak run --command=…` for store builds), so a
  PhotoCraft document opens in PhotoCraft. When an application cannot start one
  here, the row says so — "Blender Scene created empty — Blender could not start one
  here" — rather than leaving an empty file wearing an extension. The folder's own
  right-click *Create New* offers the same rows, flat, because it is already a menu.
* **A Type column in details**, between the name and the size, sorted ascending and
  descending like the others by clicking its header. Its words and the sort's words
  are the same function, so they cannot disagree, and the column gives way entirely
  on a window too narrow to hold it instead of squeezing the name to thirty pixels.
* **A document's icon carries its type.** The page draws the extension — PCRAFT,
  VECTORCRAFT, BLEND — in letters sized to the tile: measured with the real font and
  solved against the page's own band, 8.6 px for VECTORCRAFT on a 96 px tile, capped
  at 15 px on the picture wall, and no label at all below a 6 px floor where it would
  be a smudge. Two documents made by two applications no longer wear the same blank
  page.
* **Check for Updates…** in the app menu, above *About*: when you pick it, it asks
  GitHub for the newest release, offers it with its notes, and *Install and Restart*
  installs it and comes back on the new build with the same folder and flags. It is
  never automatic — nothing looks for a release until you ask — the download is
  checked against the release's sha256 before anything is replaced, and a bad
  checksum is refused with the build you have left alone. This is the one thing in
  the program that uses the network.
* **Maximising is the window manager's business.** On a two-screen desktop the
  startup fit compares the window's extent with the monitor's size instead of judging
  its far edge against the monitor's width, so a window already usable on the second
  screen is left where it is; a maximized window is not measured, resized or saved,
  and the watch for a misplaced window ends at the first press on the title row, the
  window buttons or a resize edge.
* **The guide card is gone** at your request: *What the Two Buttons Do…*, its Help
  group, F1 and `--guide` with it. The app menu is five groups and ends on *Sort by*,
  with *Check for Updates…*, *About DolphinCraft* and *Quit* pinned at the bottom.
* **About** names the release this build came from — 0.1.7 — and says it dials out
  only for *Check for Updates…*, when asked.
* 155 built-in checks (`dolphincraft --selftest`), all passing — the new ones cover
  the document-type records and the Flatpak exports, the Type column's widths and
  both sort directions, the size solved for an extension on a page, the release
  reply, its numeric version comparison and a real release archive installed over a
  fixture binary (executable, no staging file left), and the two-screen window fit.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.
From 0.1.7 on, the menu's *Check for Updates…* does the same job without the shell command.

**Archives** — `dolphincraft-0.1.7-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.7-x86_64-linux.tar.gz
./dolphincraft-0.1.7-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine. The only request DolphinCraft ever makes is the
update check, and only when *Check for Updates…* is picked.
