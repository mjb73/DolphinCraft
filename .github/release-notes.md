DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.6

* **The tab strip is the window's title bar** — one line of chrome where there were
  two. The window's name sits at the left end, the tabs follow it, and the minimize,
  maximize and close buttons sit at the right end; the toolbar below belongs to the
  tab in front, the way a browser's address bar does. Drag the bar (or any tab) to
  move the window, double-click its empty space to maximize or restore, and pull its
  edges to resize — the window draws its own one-pixel frame now that the system's
  title bar is gone. A tab keeps its own folder, sort, view and picture mode, so
  details in one and the picture wall in the next do not argue, and all of it comes
  back next session in the order it was left. The tab in front sits on a lighter
  plate than the strip behind it, so it reads as raised even before the blue line
  under it is noticed. Right-clicking a tab offers *Close Tab*, *Close Other Tabs*,
  *Close Tabs to the Right*, *Duplicate Tab* and *Reopen Closed Tab*, and a hover
  shows the full path behind the elided name.
* **Shortcut groups** in the places panel: right-click a shortcut and choose *New
  Group…*, then drag shortcuts onto the group or use *Move to Group*. A group folds
  with a click, shows the same flag as the shortcuts it holds, and removing it keeps
  them.
* **Create New makes the thing and asks for its name in place.** *Create New →
  Folder* and *Create New → Text File* on the view's own right-click menu — and F10,
  and the app menu's *New* group — make the folder or the empty text file at once,
  with no window in between; the name opens for editing where the item now sits,
  already selected, so typing replaces it and Enter keeps it (Escape keeps the
  suggestion). A second one takes the next free name (`New Folder`, `New Folder
  (1)`) instead of refusing. Create New's own submenu is now the width of every other
  submenu instead of stretching across the window.
* **A second click on a marked file renames it**, with the name selected in place —
  while a double click still opens it, Shift and Ctrl still extend the mark, and a
  quick second click counts as the double click it is.
* **Compact view sizes its columns from the longest name** in the listing, so a name
  is not cut off at a fixed five-font-height cell: a short folder keeps its narrow
  columns, and only a name wider than the whole view has to elide.
* **Every view keeps its own zoom**: icons, compact, details and the picture wall
  each remember their own size, so switching to Details does not shrink the grid you
  sized for photographs, and the wall's thumbnail height is a memory of its own. The
  zoom keys and the −/+ buttons act on the view on show, and *View → Reset Zoom*
  (Ctrl+0) resets only that one. All four are written to your settings as you change
  them.
* **Paging keys that page.** **Home** and **End** take the first and the last item,
  **Page Down** and **Page Up** move one screenful measured from the view itself — a
  page of the picture wall is the pictures in the window, a page of details rows is
  the rows in it — and the window follows the cursor each time. In the picture viewer
  the arrows step and wrap, Page Down and Page Up step one picture, and Home and End
  take the first and the last of the folder.
* **Light mode's Shading slider** (in *View*, while Light Mode is on) deepens the
  steps between the light window's surfaces — the shortcuts panel and everything
  drawn on it, the toolbar, the hovered row, the column header, the hairlines and the
  window edge — for a display that does not show subtle tones. The file view is the
  anchor and does not move; at 100 % the panel goes from 13 levels below the view to
  39, and the sizes, dates and counts darken with the surfaces they sit on. 0 % is
  the designed palette exactly.
* **The light palette is written per role**, not flipped out of the dark one: paper
  at #fbfcfd with panel and toolbar steps you can see, a hairline that survives, a
  window frame darker than what it holds, an accent that still reads on white, a
  filled button that stays a button, and a selection band that stays a band under
  white text. `--selftest` holds both modes to the same contrast targets.
* **The details header's column rules sit on the column edges**, with six pixels of
  air either side, so "Size" no longer straddles its own line; the date column keeps
  the 66 px that "Modified", its air and its sort arrow need.
* The name field a new or renamed item opens is drawn in one place now and used by
  all three views, is sized to the name column, and closes when you press another
  row; the toast hugs its message instead of a fixed 400 px card, and a selected
  folder's icon lifts off the selection band instead of vanishing into it.
* **About** names the release this build came from — 0.1.6.
* 144 built-in checks (`dolphincraft --selftest`), all passing — the new ones cover
  both palettes and their contrast targets, the columns fitting their content, the
  per-view zoom and paging arithmetic, the Create New free-name rule, paging in the
  viewer, and the title row's colour ordering.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.

**Archives** — `dolphincraft-0.1.6-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.6-x86_64-linux.tar.gz
./dolphincraft-0.1.6-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine; nothing dials out.
