DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.3

* **Open With**, as the original has it: the first row of a file's right-click menu
  opens it in the program your desktop already prefers, *Open With ▸* lists the
  other programs that claim that kind of file, and **Other Application…** opens a
  chooser with every installed program — the ones for that type first, the rest by
  name, a text field to narrow them. Ticking *Remember this choice for all files of
  type…* writes the preference into your desktop's own `mimeapps.list`, so the first
  row changes here and everywhere else.
* **Files whose name says nothing are read**: a `Makefile`, a `Cargo.lock`, a plain
  text file with an unusual extension now offers editors instead of nothing.
* **Recently Closed Tabs** under *File*, with **Ctrl+Shift+T** to bring the last one
  back where it was; the list keeps the ten most recent, newest first.
* **The picture wall holds pictures only** — sub-folders and documents stay in the
  icon and list views, and marking, Ctrl+A, Shift+click and the arrow keys follow the
  pictures on screen, so a hidden file cannot be caught by a Delete.
* **The picture details panel starts closed**, and the recipe is read behind it;
  **P** or the info button opens it.
* **Deleting asks nothing first**: mark and press Delete, with an eight-second **Undo**
  in the toast and **Ctrl+Z** afterwards. Dolphin's three confirmation switches are in
  the app menu under *Deleting* if you want the questions back.
* Sort, view, zoom, colours, picture switches and the window's size and place are
  saved as you choose them; a settings file from an older version still opens.

* **Recent Files and Recent Locations** sit under the shortcuts in the navigation
  panel: the documents this computer last used, read from your desktop's own
  recently-used list, and the folders this window has been through.
* **The toolbar now carries two words, Find and Menu.** The Open button (and the
  arrow that listed other places) is gone — opening is a double-click, or Open and
  Open With on the right-click menu — and the app menu scrolls when the window is
  short instead of running off the bottom of the screen.
* **A start is always one view**: split view is F3 for the window you are in and is
  no longer brought back on the next launch.
* **About** names the release this build came from — 0.1.3.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.

**Archives** — `dolphincraft-0.1.3-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.3-x86_64-linux.tar.gz
./dolphincraft-0.1.3-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine; nothing dials out.
