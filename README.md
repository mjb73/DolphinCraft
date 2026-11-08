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
release. Set `PREFIX` to install elsewhere, or `DOLPHINCRAFT_VERSION=v0.1.5` to pin a
release.

The tarballs are built on Debian 12 on purpose, so the binary asks for glibc 2.35 — Debian 12,
Ubuntu 22.04+, Mint 21+, Fedora 36+ and newer. A Wayland or X11 session plus an OpenGL driver
(libglvnd + mesa) is all it needs at runtime.

## What it does

- **Places sidebar**: Home, Desktop, Documents, Downloads, Music, Pictures,
  Videos, Trash, plus removable drives and your own shortcuts.
  Shortcuts can be given a colour, renamed, reordered by dragging, and removed.
- **Tabs** with a path bar you can click through, and an address field (Ctrl+L).
  Closing a tab is never final: *File → Recently Closed Tabs* lists the ten
  folders closed since the window opened, newest first, and **Ctrl+Shift+T**
  brings the last one straight back where it was.
- **Three view modes**: icons, details and columns, each with its own zoom
  level and sort order. The measured ladder is 0–16 zoom steps (16–256 px);
  the icons and compact views continue the same ladder to level 25 = 400 px.
- **Video previews**: a video shows its **first frame** wherever a picture shows
  itself — the icon grid, the compact and details rows, and the picture wall —
  instead of a generic film icon. The frame is taken by `ffmpeg` (the one your
  machine already has for video) and scaled down to the tile before it is read,
  so a 4K film costs about as much as a photo. Without `ffmpeg` installed the
  item simply keeps its kind icon; thumbnails are cached the same way a
  picture's are, so a folder of films fills once and stays filled.
- **Picture controls in the file view**: above the icon grid of any folder that
  holds pictures, a one-row strip carries the thumbnail size slider (50–400 px
  in the picture wall, the same 16–400 px ladder here), plus switches for
  **Names**, **Dimensions**, **Rounded** corners and **Previews**. The same
  switches are in the app menu under *Pictures*.
- **Find in Files** (Ctrl+Shift+F): the find bar filters *names*; this window
  reads inside them. Three modes worded the way Dolphin words them — *File
  names*, *File names and contents*, *File contents* — with a *Case sensitive*
  switch and an *Include binary files* switch, one row per file carrying the
  line the phrase was written on, and **Show** / **Open** beside every row. It
  never follows links, leaves hidden folders out, skips anything over 4 MB, and
  its summary line says exactly what it passed over.
- **Picture wall**: a wall of picture cards from 50 to 400 px. Every thumbnail is
  cut to its picture's own shape and every row is packed at exactly the size on
  the slider — the last row too, so no row jumps out bigger than the rest — with
  the same 12 px gap between pictures instead of each reserving a square with a
  band of empty wall around it. Rounded corners, an optional size line under each
  name (width × height, aspect ratio, megapixels), drawn in the file name's own
  font, size and colour, and a switch to hide the file
  names. Enter it with the picture button in the toolbar. The wall holds the
  folder's pictures and nothing else: sub-folders and documents stay in the icon
  and list views, and the path bar above the wall is how you step into a
  sub-folder. Marking, Ctrl+A, Shift+click and the arrow keys follow the pictures
  on screen, so a hidden document can never be caught up in a selection.
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
  showing size, type, date and a preview for the current selection. The window
  always starts as one view; **F3** (or the menu switch) splits it, and the
  next start is one view again.
- **Search** inside the current folder, with the results in place of the list.
  *In subfolders* looks through everything below as well, and while it does the
  details view gains a **Path** column between the name and the size, so results
  from different folders can be told apart — the folder is written relative to
  the one you searched, and a file sitting in that folder itself shows a dash.
- **Added tools**
  - The sidebar keeps them in one **Added tools** group at the bottom of the
    panel, folded until you click its heading; the heading counts the rows so you
    can see what is inside without opening it, and it remembers being opened.
  - **Find Duplicates** — groups identical files by content hash.
  - **Bulk Rename** — renames many files at once from a pattern.
  - **Quick Look** — a fast preview window for a single file (Space).
  - **Open Terminal Here** — a shell in the folder you are looking at.
  - **Manage Disk Space Usage** — adds up everything below the folder and lists
    what takes the room, largest first, with a *Free Up Disk Space* shortlist of
    the biggest files and a **Show** button for each. The original hands this
    question to Filelight and offers to install it; this window answers it
    itself, so nothing has to be installed. Hidden files count, a symbolic link
    is never followed, and a folder you cannot read is reported instead of being
    quietly left out.
  - **Compress…** — marks items (or the folder on show) and turns them into one
    ZIP in the folder they came from, with the name filled in and editable
    first. It writes `photos.zip` for one folder, `Archive.zip` for several
    items, and steps aside to `photos (1).zip` rather than overwrite anything.
  - **Extract…** — unpacks a ZIP or a tar archive into a folder named after the
    file (or into this folder, if you untick *Into a folder of its own*) and
    takes you there afterwards. Both archive tools use the `zip`, `unzip` and
    `tar` programs your computer already has, and say so plainly when one is
    missing.
  - **Resize Image…** — right-click one picture, or a whole selection of them,
    and give a size in **megapixels** (1, 2, 4, 8, 12, 24 with one click, or any
    number typed in). The width and the height are scaled by the same factor, so
    each picture keeps its own shape; the window shows the size it will come out
    at while you type. A copy is written beside the original as
    `name-2MP.jpg` (stepping aside to `name-2MP (1).jpg` rather than overwriting
    anything), or the picture itself is replaced if you tick that box. PNG, JPEG,
    WebP, BMP and TIFF are written; JPEG gets a quality slider. It works from the
    right-click menu, from *Tools → Resize Image…*, and from the side panel's
    *Added tools* group.
  - **Drag and drop, both ways** — a file or a picture dragged out of the window
    is handed to whatever program takes it (a chat window, a browser, an image
    editor, another file manager): the drop carries the usual `text/uri-list`,
    so the other program gets the file itself, not a copy of its contents. A
    selection goes in one drag, and a long list — hundreds of pictures — is
    transferred in chunks rather than cut short. Dragging *into* the window
    copies the files into the folder on screen: a wash names that folder while
    the drag is held over it, a name already taken steps aside, and nothing is
    ever overwritten. On a Wayland session the window opens as an X11 one so both
    drags can reach the desktop (`DOLPHINCRAFT_BACKEND=wayland` keeps the
    compositor's own path at the cost of them).
- **Copy to Other View / Move to Other View** (**Shift+F5** / **Shift+F6**), and
    **Copy To… / Move To…** from the right-click menu: the marked items go into the
    view beside this one, or into a folder picked in a small window with a field to
    narrow the list. Both refuse in the original's own words — nothing marked, the
    other view already holding them, a folder that cannot be written, a move out of
    a folder that cannot be written — and neither ever overwrites: a name already
    taken steps aside to `name (1)`.
- **Open With**, the way the original does it: right-click a file and the first
  row opens it in the program your desktop already prefers, *Open With ▸* lists
  the other programs that claim that kind of file, and **Other Application…**
  opens a chooser with every installed program — the ones for that type first,
  the rest by name, a text field to narrow them. Ticking *Remember this choice
  for all files of type…* writes that preference into your desktop's own
  `mimeapps.list`, so the first row changes here and in every other program. A
  file whose name says nothing (a `Makefile`, a `Cargo.lock`) is read, so editors
  are offered for it instead of nothing.
- **Network group** in the sidebar, where the original's *Remote* group is: it
  lists the network folders this computer already has — SMB, NFS and SSH shares in
  the mount table, and shares opened through the desktop's own file choosers —
  each named after its share. Nothing is mounted from here; with none mounted it
  says *No shared folders found*, and the heading folds away. A computer with no
  shares at all can take the group off the panel altogether: the app menu's
  **View** group carries *Show Network Shared Folders*, so the heading and its rows
  leave together, and the same switch brings them back.
- **Light mode**, from the app menu's **View** group: *Light Mode* switches the
  whole window — view, panel, toolbar, dialogs, icons, text — from the dark
  palette to a light one, and back. The light palette is derived from the dark
  one by flipping its lightness, so every colour keeps its hue (folders stay
  blue, the accent stays blue) and anything added later is right in both. Your
  choice is remembered; a window that has never been switched stays dark, which
  is how it has always looked.
- **Two recent groups in the sidebar**, under your shortcuts: **Recent Files**
  lists what you have actually opened lately, newest visit first, read from the
  desktop's own recently-used list — a row opens the file, its menu can show it
  in its folder or copy its path, and files that have since been moved or deleted
  are left out rather than offered as dead rows. **Recent Locations** remembers
  the six folders you have walked into this window, newest first, so the way back
  to a deep folder is one click; either heading folds away when you don't need it.
- Copy, cut, paste, rename, move to trash, delete, new folder, and file
  properties, all with the shortcuts you already know from Dolphin.
- **Create New, on the folder itself**: right-click the empty space in the view
  — or anywhere in a folder that has nothing in it yet — and the menu offers
  *Create New → Folder…* and *Create New → Text File…*. Both open one small
  window with the suggested name already selected, so typing replaces it; the
  folder can be renamed in place straight after, and the text file is made empty
  and opens in your editor from there. F10 still makes a folder, and the app
  menu's *New* group carries *New Text File…* too.
- **Deleting asks nothing first** (as you asked): mark the pictures — or any
  files — and press Delete; they are in the Trash before you let go of the key.
  Shift+Delete removes them for good the same way. What replaces the question is
  a way back: the toast at the bottom offers **Undo** for eight seconds, and
  **Ctrl+Z** or the app menu's *Undo Move to Trash* still brings the items back
  afterwards. If you ever want the question back, all three of Dolphin's
  confirmation switches live in the app menu under *Deleting*.

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
--guide                   open the card that explains Find and Menu
--menu                    open the app's own menu (used to photograph it)
--open-with <file>        open the Open With chooser on that file at once
                          (used to photograph it)
--trash-demo <folder>     move that folder's contents to the Trash once, so the
                          Undo toast can be photographed; nothing is deleted for good
--space                   open the disk-usage window at once
--compress                open the archive window on the folder at once
--extract <archive>       open the unpack window on that archive at once
--resize <picture>        open the Resize Image window on that picture at once
                          (used to photograph it)
--drag-out <file>...       hand those files to another program as a drag, for
                          checking the hand-over without a mouse
--size <WxH>              open the window at a given size
--light                   open in the light palette for this run, without changing
                          the saved setting
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
| `src/imageresize.rs` | the Resize Image window's engine: megapixels in, same shape out |
| `src/dragout.rs` | the drag out of the window: XDND spoken to the X server directly |
| `src/search.rs` | searching a folder and filtering the results |
| `src/fsutil.rs` | file operations, sizes, dates, thumbnails, trash |
| `src/config.rs` | reading and writing the settings file |
| `src/selftest.rs` | the app's own checks (`--selftest`) |