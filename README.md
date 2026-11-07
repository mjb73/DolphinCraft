# DolphinCraft

A desktop file manager written in Rust with [egui](https://github.com/emilk/egui).
It rebuilds the everyday experience of the KDE file manager Dolphin — places
sidebar, tabs, breadcrumb path bar, icon and detail views, zoom, sorting,
drag and drop, the information panel — and adds a few tools of its own.

## Download

Prebuilt Linux binaries are on the [releases page](https://github.com/mjb73/DolphinCraft/releases):
a tarball and an identical `.zip` per architecture (x86_64, aarch64), each with a `.sha256`.

```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```

`install.sh` needs no root: it unpacks the release into `~/.local/bin/dolphincraft`, writes
`~/.local/share/applications/dolphincraft.desktop` with an absolute path, and refreshes the
release. Set `PREFIX` to install elsewhere, or `DOLPHINCRAFT_VERSION=v0.1.2` to pin a
release.

The tarballs are built on Debian 12 on purpose, so the binary asks for glibc 2.35 — Debian 12,
Ubuntu 22.04+, Mint 21+, Fedora 36+ and newer. A Wayland or X11 session plus an OpenGL driver
(libglvnd + mesa) is all it needs at runtime.

## What it does

- **Places sidebar**: Home, Desktop, Documents, Downloads, Music, Pictures,
  Videos, Trash, plus removable drives and your own shortcuts.
  Shortcuts can be given a colour, renamed, reordered by dragging, and removed.
- **Tabs** with a path bar you can click through, and an address field (Ctrl+L).
- **Three view modes**: icons, details and columns, each with its own zoom
  level and sort order. The measured ladder is 0–16 zoom steps (16–256 px);
  the icons and compact views continue the same ladder to level 25 = 400 px.
- **Picture controls in the file view**: above the icon grid of any folder that
  holds pictures, a one-row strip carries the thumbnail size slider (50–400 px
  in the picture wall, the same 16–400 px ladder here), plus switches for
  **Names**, **Dimensions**, **Rounded** corners and **Previews**. The same
  switches are in the app menu under *Pictures*.
- **Picture wall**: a wall of picture cards from 50 to 400 px. Every thumbnail is
  cut to its picture's own shape and every row is packed at exactly the size on
  the slider — the last row too, so no row jumps out bigger than the rest — with
  the same 12 px gap between pictures instead of each reserving a square with a
  band of empty wall around it. Rounded corners, an optional size line under each
  name (width × height, aspect ratio, megapixels), drawn in the file name's own
  font, size and colour, and a switch to hide the file
  names. Enter it with the picture button in the toolbar; folders sit in the same
  wall.
- **Picture viewer and slideshow**: open a picture and page through the folder,
  zoom, and read the generation prompt, negative prompt, steps, sampler, CFG
  scale, seed, size, model and denoising strength straight out of the file
  (PNG text chunk or JPEG comment). The prompt can be selected with the mouse —
  a drag that reaches the edge of its box keeps scrolling and keeps selecting,
  so a long prompt can be copied whole; the wheel scrolls it without losing the
  selection, Shift+click extends, Ctrl+A and Ctrl+C work, and the right-click
  menu offers Select all and Copy selection. The whole block folds away behind
  its own title row — one click, or **P** in the viewer — leaving a single line
  instead of the full recipe, and it remembers which way you left it. The
  details panel itself starts **closed**: the picture comes up clean, and the
  info button in the bar, **I**, or **P** opens it when you want it. The
  slideshow fills the screen,
  moves on every 1.5 seconds to start (a number you can change while it plays)
  and cross-fades over 300 ms; the fade can be switched off.
- **Compare two pictures** side by side, split or overlaid, with synced zoom.
  Each picture zooms where the mouse is pointing and can be dragged to its edges
  but never out of its half; **Sync** locks the two together (Split and Overlay
  are always locked), and the read-out shows one percentage, or two once they are
  separate.
- **Sorting** by name, size, type or date, with folders-first and reverse options.
- **Split view** for two folders side by side, with the information panel
  showing size, type, date and a preview for the current selection.
- **Search** inside the current folder, with the results in place of the list.
- **Added tools**
  - **Find Duplicates** — groups identical files by content hash.
  - **Bulk Rename** — renames many files at once from a pattern.
  - **Quick Look** — a fast preview window for a single file (Space).
  - **Open Terminal Here** — a shell in the folder you are looking at.
- Copy, cut, paste, rename, move to trash, delete, new folder, and file
  properties, all with the shortcuts you already know from Dolphin.

## Running it

You need a Rust toolchain (`rustup` from <https://rustup.rs>) and a Linux
desktop session with either X11 or Wayland.

```
cargo run --release
```

To build a standalone binary and put it on your application menu:

```
cargo build --release
```

then copy `dolphincraft.desktop.example` to `~/.local/share/applications/dolphincraft.desktop`
and change the two paths in it to where `target/release/dolphincraft` ended up on
your machine. `start.sh` does the same thing in one step:

```
./start.sh          # builds if needed, then launches the app
```

Options:

```
--screenshot <file.png>   draw a few frames, save a picture of the window and exit
--dir <path>              start in a folder instead of the remembered one
--view icons|compact|details   start in a given view
--zoom <0-16>             start at a given zoom level
--pictures               start in the picture wall
--thumbs <50-400>        start the wall at a given thumbnail size
--dims                   start the wall with the size line showing
--viewer <picture>       open a picture in the viewing window
--details                open the viewer's details panel at once (it starts closed)
--play                   start the viewer's slideshow
--compare side|split|overlay   open the comparison beside the viewer
--keep-settings           use the saved layout instead of factory settings
--find <text>             open the find bar with these words already narrowing the list
--deep                   with --find, look through every folder below as well
--guide                   open the card that explains Find, Open in and Menu
--menu                    open the app's own menu (used to photograph it)
--size <WxH>              open the window at a given size
--selftest                run the built-in checks and exit
```

## Where the numbers come from

`MECHANICS.md` is the measurement sheet: the zoom ladder, view defaults, layout
margins, status-bar wording and other constants, each with the function address
it was read from in the original program. `mechanics.json` repeats the same
values as data for the build checks. Where something could not be measured it is
marked as estimated, and the reason is written next to it.

## Project layout

| file | what lives there |
| --- | --- |
| `src/main.rs` | the window, the keyboard handling, the main view and the entry point |
| `src/state.rs` | what survives a restart: tabs, folders, view settings, shortcuts |
| `src/views.rs` | drawing the file views (icons, details, columns) |
| `src/theme.rs` | colours, icons drawn in code, fonts |
| `src/panels.rs` | sidebar, information panel, menus, Quick Look, Bulk Rename, duplicates |
| `src/search.rs` | searching a folder and filtering the results |
| `src/fsutil.rs` | file operations, sizes, dates, thumbnails, trash |
| `src/config.rs` | reading and writing the settings file |
| `src/selftest.rs` | the app's own checks (`--selftest`) |