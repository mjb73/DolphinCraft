DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.5

* **Light mode**, from the app menu's *View* group: *Light Mode* switches the whole
  window — view, side panel, toolbar, tabs, dialogs, icons, text — from the dark
  palette to a light one, and back. Your choice is remembered, and a window that
  has never been switched stays dark, exactly as it has always looked. The light
  colours are derived from the dark ones (hue kept, lightness flipped), so folders
  stay blue and anything added later is right in both modes; the document icon
  keeps its paper page, now with a grey edge so every file type still has a shape
  on a white view. `--light` opens one window light without touching the setting.
* **Resize Image…** — right-click one picture or a whole selection and give a size
  in megapixels, the unit the original's own caption prints. Width and height scale
  by one factor, so the shape is kept to the pixel, and the window shows the size it
  will come out at while you type. A copy is written beside the original
  (`name-2MP.jpg`, stepping aside rather than overwriting) unless *Replace* is
  ticked. PNG, JPEG, WebP, BMP and TIFF, a quality slider for JPEG, and
  Exif-turned photographs saved upright.
* **Drag and drop, both ways**: a drag out of the window hands the marked files to
  any other program, and a drag into it copies its files into the folder on show —
  a wash names that folder while the drag is held over it, a name already taken
  steps aside, and the Trash refuses drops. Both directions need X11, so a Wayland
  session that also offers X (XWayland) opens the window as an X11 one;
  `DOLPHINCRAFT_BACKEND=wayland` opts out.
* **Copy to Other View / Move to Other View** (**Shift+F5** / **Shift+F6**) and
  **Copy To… / Move To…** in the right-click menu: the marked items go into the view
  beside this one, or into a folder chosen in a small window. The refusals are the
  original's own, in its words, and a name already taken steps aside rather than
  being overwritten.
* **A video shows its first frame** wherever a picture shows itself — icon grid,
  compact and details rows and the picture wall — taken by ffmpeg and cached like
  any other thumbnail, scaled to the tile before it is read. No ffmpeg is not an
  error: the item keeps its film icon.
* **A search's columns fit what they hold**: the *Name* and *Path* columns are
  measured from the longest name and the longest folder in the result, so neither
  is cut off while there is room to show it, and *Size* and *Modified* give back
  their spare width first when the row is tight. A long name and a deep folder now
  both read in full at an ordinary window width.
* **The find bar stays whole**: its close button, the found count and *Stop* are
  placed against the right edge first and the field takes what is left, so the ✕ no
  longer falls off the window, and it is drawn as a line glyph rather than the `✕`
  character, which this window's font does not carry.
* **About** names the release this build came from — 0.1.5.
* 134 built-in checks (`dolphincraft --selftest`), all passing — the new ones cover
  both palettes and what they must not be used for, the search columns fitting their
  content, the find bar's close button staying inside its row, and the video frame,
  drag and drop, resize and the copy/move refusals above.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.

**Archives** — `dolphincraft-0.1.5-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.5-x86_64-linux.tar.gz
./dolphincraft-0.1.5-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine; nothing dials out.
