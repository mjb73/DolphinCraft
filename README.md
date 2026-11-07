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
  level and sort order (0–16 zoom steps, 16–256 px icons).
- **Picture wall**: a grid of picture cards from 50 to 400 px, with rounded
  corners, an optional size line under each name (width × height, aspect ratio,
  megapixels) and a switch to hide the file names. Enter it with the picture
  button in the toolbar; folders sit in the same grid.
- **Picture viewer and slideshow**: open a picture and page through the folder,
  zoom, and read the generation prompt, negative prompt, steps, sampler, CFG
  scale, seed, size, model and denoising strength straight out of the file
  (PNG text chunk or JPEG comment). The slideshow fills the screen, moves on
  every N seconds (a number you can change while it plays) and cross-fades over
  300 ms; the fade can be switched off.
- **Compare two pictures** side by side, split or overlaid, with synced zoom.
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
--play                   start the viewer's slideshow
--compare side|split|overlay   open the comparison beside the viewer
--keep-settings           use the saved layout instead of factory settings
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