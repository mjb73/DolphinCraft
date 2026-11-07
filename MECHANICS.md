# What the original program does — measured, not guessed

Target: `/usr/bin/dolphin` (KDE Dolphin, 26.08.2) and its private library
`/usr/lib/libdolphinprivate.so.6` (26.08.2). The main binary is stripped, but the
shared library keeps its dynamic symbol table, so the functions below have real
names and addresses. Addresses are file offsets inside `libdolphinprivate.so.6`
(identical to virtual addresses in this ELF).

Two kinds of evidence were used:

1. Disassembly of the named functions (objdump on the installed library).
2. A live capture of the installed program in the user's KDE session
   (dark Breeze theme, Details view, then Split View with the Information panel).
   Screenshots: `/tmp/dolui/shot2.png`, `/tmp/dolui/split2.png` (throw-away look,
   not part of the deliverable).
3. User-interface text extracted from the binary (`strings /usr/bin/dolphin`) and
   from the library, cited by the exact string where it matters.

## Zoom ladder — fully measured

`ZoomLevelInfo` (all in libdolphinprivate.so.6):

| function | address | result |
| --- | --- | --- |
| `ZoomLevelInfo::minimumLevel()` | 0x1a6520 | returns 0 |
| `ZoomLevelInfo::maximumLevel()` | 0x1a6530 | returns 16 |
| `ZoomLevelInfo::iconSizeForZoomLevel(int)` | 0x1a6540 | level 0..4 read a table, level >4 returns `level * 16` |
| `ZoomLevelInfo::zoomLevelForIconSize(QSize)` | 0x1a6560 | inverse: 16→0, 22→1, 32→2, 48→3, 64→4, then `4 + (size-64)/16` |

The table at 0x1cd980 is five 32-bit ints: `0x10, 0x16, 0x20, 0x30, 0x40`, i.e.
**16, 22, 32, 48, 64 px**. Levels 5..16 are 80, 96, 112, … 256 px. So the icon
size ladder used by the rebuild is exactly:

    16, 22, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256

and the zoom range is 0..16 (17 steps). This is what `app/src/state.rs` stores
per view mode, and `mechanics.json` repeats as data.

## View modes and per-URL view properties

* `ViewProperties::ViewProperties(QUrl const&)` @ 0x19fdd0 loads saved properties
  for the URL (per-folder `iconSize`, `viewMode`, `sortRole`, `sortOrder`,
  `previewsShown`, `hiddenFilesShown`; keys visible in the library's string pool:
  `iconSize`, `sortRole`, `sortOrder`, `previewsShown`, `hiddenFilesShown`,
  `sortFoldersFirst`, `groupedSorting`, `zoomLevel`).
* `DolphinView::updateDefaultZoomLevel()` @ 0x17df20 picks the default zoom from
  the mode settings (`IconsModeSettings`, `CompactModeSettings`,
  `DetailsModeSettings`) and converts an icon size into a zoom level through
  `ZoomLevelInfo::zoomLevelForIconSize`. Which *default* icon size each mode uses
  is a KConfigSkeleton default that is inlined into local (hidden) constructors,
  so it is **estimated** below: Details 16 px (level 0), Icons 64 px (level 4),
  Compact 16 px (level 0). The user's own `~/.config/dolphinrc` has
  `[DetailsMode] IconSize=256` and `PreviewSize=80`, which is a personal choice,
  not a factory default, so it was not copied.
* `DolphinView::setZoomLevel(int)` @ 0x1715b0, `ViewProperties::setZoomLevel(int)`
  @ 0x19cfc0, and `DolphinViewActionHandler::slotZoomLevelChanged` @ 0x193ac0
  clamp a zoom change to 0..16 and push the resulting icon size into the view.

## What the window contains (live capture + strings)

Main window, dark Breeze theme:

* Top row: application button, Back / Forward / Up, the location bar showing
  clickable breadcrumb segments (`root > usr > share > applications`, current
  segment bold), then on the right: split-view button, filter, search, menu.
* Tab strip directly under the toolbar: one folder icon + label per tab, active
  tab marked with the accent colour, `+` button at the end. Dolphin's own strings
  include "Open in New Tab", "Open in New Tabs", "Close Tab", "Go to Tab %1",
  "Empty Recently Closed Tabs", "Undo close tab".
* Left: Places panel with sections `Places`, `Remote`, `Recently Saved`,
  `Devices` (all exact strings in the binary). Entries: Home, Desktop, Documents,
  Downloads, Music, Pictures, Videos, Trash, user bookmarks, mounted devices.
* Centre: one or two views (Split View, F3). In split mode **each view carries
  its own location bar** (seen in the capture).
* Right: Information panel (F11) — large preview, the item name centred, a
  divider, then `Type:`, `Size:`, `Modified:`, `Accessed:` rows in that order
  (capture), and a Close button in its header.
* Bottom: status bar per view. The line is composed from plural message ids
  (`1 folder` / `%1 folders`, `1 file` / `%1 files`, `%1 (%2)`, `%1, %2 (%3)`,
  `0 folders, 0 files`) — measured in the code, see the decompiler section
  below; the capture showed `204 files (1718.4 KiB)`. `%1 items selected`
  belongs to the Information panel, and `%1 free out of %2 (%3% used)` is the
  disk-space button's tooltip (bar text `%1 free`); a zoom slider sits at the
  right edge (`Show Zoom Slider`).

Details view columns out of the box: `Name`, `Size`, `Modified` (capture).
Sorting strings: `Natural`, `Alphabetical, case sensitive`, `Ascending`,
`Descending`, `Show folders first`, `Show hidden files`, `Show preview`,
`Show in groups`.

Placeholders/messages, exact: `Folder is empty`, `The location is empty.`,
`No items found.`, `No items matching the filter`, `No items matching the search`,
`Trash is empty`, `Loading folder`, `Searching`, `Trash operation completed.`,
`Delete operation completed.`, `Copy to Other View`, `Move to Other View`,
`Compare Files`, `Manage Disk Space Usage`, `Open Terminal`, `Copy Location`,
`Duplicate Here`, `Create Folder`, `Create File`, `Add to Places`,
`Show Hidden Files`, `Restore to Former Location`, `Empty Trash`.

Confirmations tab strings: `Emptying trash`, `Deleting files or folders`,
`Opening many folders at once`, options `Always ask`, `Open in application`,
`Run script`. The rebuild asks before destructive deletes (see below).

Double-click behaviour is configurable in the original (`Double-click triggers`,
`By clicking on icon or name`); the rebuild always opens on double-click.

## Behaviour rules carried into the rebuild

* Navigation history is per view; Back/Forward/Up (Alt+Left / Alt+Right /
  Alt+Up), F5 refresh, Ctrl+L makes the location bar editable, Ctrl+H toggles
  hidden files, F3 splits, F11 toggles the Information panel, F9 toggles Places,
  F10 creates a folder, F2 renames inline, Del moves to Trash, Shift+Del deletes
  permanently.
* Trash lives at `~/.local/share/Trash/{files,info}` with a `.trashinfo` record
  per item holding `[Trash Info] Path=` and `DeletionDate=` — the freedesktop
  convention Dolphin uses (strings `Restore to Former Location`, `Empty Trash`,
  `Configure Trash`). The rebuild writes the same format.
* Size text is IEC with one decimal (`1718.4 KiB`), as in the capture.
* The file list writes absolute short dates, `dd/MM/yyyy HH:mm`
  (`06/11/2026 18:43`), even for files changed minutes earlier — the capture
  shows the numeric form, not "Today at", so relative wording is not used there.
* The Information panel uses the relative wording from the capture
  (`14 minute(s) ago` for something just touched), falling back to the numeric
  date for older items. The Properties dialog keeps the long form
  (`Friday, 1 May 2026 at 13:00`).

## Added features (request: "same app but with added features")

These do not exist in the original and are labelled as additions in the UI:

1. Quick Look overlay (Space) — large preview of the selected image or text.
2. Search panel with filters (name, minimum size, age) and a result count.
3. Find Duplicates — groups identical files by size then SHA-256, with the
   wasted space total.
4. Bulk Rename — pattern with `{name} {n} {ext} {date}`, live old → new preview,
   collision check.
5. Checksums in Properties (SHA-256, computed on request).
6. Folder size on demand, plus "Size of contents" in the status bar.
7. Folder colours — a right-click colour swatch on a folder, stored in a small
   hidden file inside that folder (`.directory`), so the tint survives copying
   the folder and needs no database. Icons and the Information panel honour it.
8. Address bar that accepts a pasted path or a `file://` link, expanding `~` and
   opening a named file's folder with that file selected.
9. A built-in self-check (`--selftest`) that runs the file operations behind
   these tools against a throw-away folder and a throw-away home, so trash
   records, duplicate grouping, bulk rename and search can be verified without
   touching the person's own files.
10. Picture wall — a grid of picture cards (with folders beside them) replacing
    the list, thumbnail size 50..400 px, rounded corners, hide-names and
    show-dimensions switches. Values read from the person's image browser; see
    the section at the end of this sheet.
11. Full-window picture viewer with zoom, pan and a Prompt panel that reads the
    generation settings out of the picture file.
12. Full-screen slideshow with an interval in seconds and a 300 ms cross-fade,
    both changeable while it plays.
13. Side-by-side picture comparison, with the browser's three modes (side,
    split, overlay), synced zoom and a swap.
14. **No question before a delete, with Undo instead** (asked for by the person:
    "When I mark images and press delete, I don't want another confirmation
    step"). Delete on a marked selection moves it to the Trash and Shift+Delete
    deletes it for good, both at once. A Move to the Trash leaves a toast with an
    **Undo** button for eight seconds, and **Ctrl+Z** or the app menu's *Undo Move
    to Trash* puts the items back after that. The original's three confirmation
    switches are kept in the app menu under *Deleting*, off by default for the two
    the person asked about and on for emptying the Trash.

## Thumbnails follow the zoom level (asked for by the person)

The person reported that the `+` button at the bottom right raises the resolution
but the thumbnails of pictures and other files stay the same size. In the original
the picture always grows with the zoom, and that is measured, not assumed.

Code in `libdolphinprivate.so.6`:

| what | address | result |
| --- | --- | --- |
| `DolphinItemListView::setZoomLevel(int)` | 0x16e100 | clamps the level to 0..16 (the pair of bounds is packed at 0x1c3350), asks `ZoomLevelInfo::iconSizeForZoomLevel` @0x1a6540 for the pixel size, stores that size at object offset +0x200 when previews are on and at +0x204 when they are off, writes the level to +0x1fc, then calls `updateGridSize` |
| `DolphinItemListView::updateGridSize()` | 0x16db80 | picks the visible size: +0x200 when `previewsShown()` is true, otherwise +0x204; falls back to the per-mode settings object (`DetailsModeSettings` +0x2c `IconSize`, +0x30 `PreviewSize`; the icons and compact modes have their own) when the stored size is 0; converts the size back with `ZoomLevelInfo::zoomLevelForIconSize` and lays the grid out again |
| `KStandardItemListWidget::drawPixmap(QPainter*, QPixmap const&)` | 0x135bb0 | divides the cached preview's width by its device pixel ratio, compares the result with the cached target width and height kept at +0x1b0 and +0x1b4, and when they differ copies the pixmap, calls `KPixmapModifier::scale` @0x15e820 with the icon box size, restores the ratio and paints it centred |
| `KPixmapModifier::scale(QPixmap&, QSize const&)` | 0x15e820 | scales the pixmap to fit the box, keeping the aspect ratio |

So the rule the rebuild follows:

* the zoom level (0..16) gives the icon size (16, 22, 32, 48, 64, then 80..256 in
  steps of 16, see the ladder above);
* a preview fills a square of that icon size, centred, aspect kept, and it is both
  enlarged and shrunk to do so;
* a cached preview is re-scaled whenever the icon size changes, so the `+` button
  always makes the picture bigger, at every level;
* previews are drawn from zoom 0 upwards — the original shows 16 px picture
  thumbnails in its details view, so the rebuild no longer hides them below 32 px;
* the size is stored per view mode, as the three mode settings objects are.

Live evidence (new folder, `checks/orig-zoom/`): dolphin 26.08.2 in an isolated
session on a virtual display, opened on a folder with three real PNGs and a text
and CSV file.

* `icons-64px.png` — Icons view at zoom 4: 64 px picture thumbnails.
* `icons-160px.png` — after six `Ctrl+Plus` steps: the same thumbnails are 160 px
  wide, sharper, and the text and CSV files' type icons grew with them.
* `details-16px.png` — Details view at zoom 0: 16 px picture thumbnails inside the
  rows, with the type icons of the non-picture files at the same size.

What the rebuild does (`app/src/search.rs`, `app/src/views.rs`):

* `ThumbCache` remembers, per file, the pixel size its texture was rendered at. A
  request that wants a bigger picture than the one held queues a new render and
  keeps showing the old texture until the new one arrives, so no cell goes blank
  while the zoom changes. A texture that is already big enough is reused and
  drawn scaled down. Before this change the cache was keyed by path alone, so the
  first (small) thumbnail was reused at every later zoom level and the picture
  stayed small — which is exactly what the person saw.
* the request size is the icon size times the screen scale, so a HiDPI window gets
  a crisp thumbnail.
* `draw_preview` in the views draws a texture into the icon box with the aspect
  ratio kept and the picture centred, up or down, the same fit rule as
  `KPixmapModifier::scale`.
* picture previews now also appear in the Details and Compact rows, not only in
  the Icons view, matching `details-16px.png`.
* `--zoom <0..16>` and `--view icons|compact|details` were added so a picture can
  be taken at a chosen zoom level; `--screenshot` still uses factory settings.
* finished previews are kept on disk in `~/.cache/file-craft/thumbs`, named after
  the file, its modification time and the pixel size — the same role as the
  original's `~/.cache/thumbnails` (KIO `ThumbCreator` cache, kept per size, which
  is why a bigger zoom step re-renders instead of stretching the old picture).
  Entries nobody has opened for two months are pruned on start.
* decoding runs on one worker per core (at most eight). A folder with 76
  pictures fills every visible tile while the window is still opening, instead
  of the first dozen only.

Checked by driving the built window on a virtual display: starting at 64 px,
six clicks on the `+` at the bottom right took the status bar to 160 px and the
thumbnails grew with it (`app/shots/zoom-before.png`, `app/shots/zoom-after.png`).
Re-checked this round at 32 → 64 px with a real cursor click on the `+`
(`app/shots/plus-before.png`, `app/shots/plus-after2.png`: status bar 32 px → 64 px,
the two pictures grew from slivers to readable screenshots), and at 64 vs 192 px
in all three layouts (`app/shots/details-z12.png`, `app/shots/compact-z12.png`,
`app/shots/i-z12.png`).
`--selftest` covers the cache rule without a window: "a bigger zoom asks for a
bigger picture" reports `64 px became 160 px`.

## Ctrl + wheel zooms, and the +/- buttons step the same way (asked for by the person)

Decompiled from `/usr/lib/libdolphinprivate.so.6` (26.08.2), so these are measured
constants, not estimates:

| function | address | what it says |
| --- | --- | --- |
| `DolphinView::wheelEvent(QWheelEvent*)` | 0x272710 | if the Control modifier bit is **not** set (byte at +0x23 & 4 — the third byte of Qt's modifier mask, i.e. `Qt::ControlModifier`) it clears the event's accept flag and returns, so a plain wheel only scrolls. With Ctrl it adds the wheel delta to a running total at `this+0xcc`; when the total passes **+0x78 = 120** it calls `slotIncreaseZoom` and subtracts 120, when it passes **−120** it calls `slotDecreaseZoom` and adds 120. The leftover stays in the total for the next event. |
| `DolphinView::slotIncreaseZoom()` | 0x284b00 | `setZoomLevel(zoomLevel() + 1)` |
| `DolphinView::slotDecreaseZoom()` | 0x284b30 | `setZoomLevel(zoomLevel() - 1)` |
| `DolphinView::setZoomLevel(int)` | 0x2715b0 | applies the level, stores it for the current URL through `ViewProperties::setZoomLevel` (or −1 when it equals the default), hides the tool tip, emits `zoomLevelChanged(old, new)`, then saves if the properties differ |
| `DolphinItemListView::setZoomLevel(int)` | 0x26e100 | clamps to 0..16 and turns the level into the icon size (see the zoom ladder) |
| `KItemListController::increaseZoom()` | 0x1f8c60 | emits `increaseZoomClicked`, which the view connects to `slotIncreaseZoom`; this is what the status bar's `+` button (and the slider's arrow steps) trigger |

So the rule copied into the rebuild (`app/src/state.rs`: `WHEEL_UNITS_PER_STEP`,
`wheel_zoom_steps`, `wheel_units`; used from `Pane::wheel_zoom_ui` in
`app/src/views.rs`) is:

* one wheel notch = 120 Qt wheel units = **one zoom level**;
* the sum of the wheel deltas is kept between events, so two half-notches also
  make a step, exactly as the original keeps the remainder in `this+0xcc`;
* Ctrl is the only qualifier tested — Ctrl+Shift and Ctrl+Alt still zoom;
* without Ctrl the wheel is ignored by the view (the list scrolls instead);
* the result is clamped to levels 0..16, i.e. 16 px … 256 px;
* the `+` and `−` in the status bar take the same single step and hit the same
  clamp, and the `NN px` label between the free-space text and the slider
  always shows the size that resulted.

egui delivers wheel input as a line-based event (a real wheel, one line per
notch) or a pixel-based one (touchpad / smooth mice). The line case is 120 units
directly; the pixel case is converted with egui's own 40 points per line.

Checked by driving the built window on a virtual display
(`app/shots/wheel-test.sh`, pictures in `app/shots/wheel/`):
start 176 px → three notches with Ctrl held → 256 px (the top of the ladder, so
the clamp is visible) → three notches **without** Ctrl → still 256 px, the list
scrolls instead → two clicks on the `−` button at the bottom right → 224 px →
one click on `+` → 240 px. `--selftest` also covers the arithmetic without a
window: "Ctrl + one wheel notch is exactly one zoom step" and "wheel zoom stops
at the smallest and biggest icon size".

Known difference: the original stores the zoom per folder (`ViewProperties` for
the current URL, and `setZoomLevel` writes −1 when the size equals the default),
while this rebuild keeps one remembered size per view mode for the whole
program, plus a separate level per view mode as before.

## Estimated (not found in the code, marked as such in mechanics.json)

* Factory default icon and preview sizes were estimated in an earlier round.
  They are now read out of the three settings constructors (Icons 32/64,
  Compact 16/48, Details 16/48) and out of `updateDefaultZoomLevel` @0x27df20 —
  see the decompiler section at the end of this sheet.
* Details row height 26 px, Icons cell padding, sidebar 208 px, Information panel
  288 px at 100 % scaling — measured off the capture at 125 % scaling and
  rounded, so they are approximations of the theme, not constants in the code.
* "Show folders first" and the sort key/order defaults are still estimates;
  the name comparison itself is no longer one — `SortingChoice` defaults to
  `NaturalSorting` (see the deep pass at the end of this sheet).

## Shortcuts in the navigation panel (added feature)

The original's navigation panel is the Places panel. Evidence for how it works:

* Class and model names in `/usr/bin/dolphin`: `PlacesPanel`, `PlacesPanelSettings`,
  `DolphinPlacesModel`, `PlacesDataSource`; actions `add_to_places` /
  `Add to Places` (tooltip text `@action:inmenu Add current folder to places`)
  and `Show Hidden Places`, `show_places_panel`.
* Bookmarks are KIO bookmarks (`libKF6Bookmarks`, `bookmarks.xml`); the panel
  help text in the binary says entries are added by dragging folders onto the
  panel or with the context menu, and that a right-click opens an entry in a
  new tab or window.
* The panel's sections in the live capture: `Places`, the person's bookmarks,
  `Network`, `Recent Files`/`Recent Locations`, `Devices`.
* The original keeps keyboard shortcuts out of the panel: a live capture of
  dolphin 26.08.2 (`checks/orig-live/live.png`) shows no key hints beside the
  rows. Shortcuts appear in menus only, and are edited in the "Shortcuts"
  (KShortcutsDialog) window — strings `Shortcuts`, `KShortcutsDialog`,
  `setDefaultShortcut`, `KStandardShortcut` (`moveToTrash`, `createFolder`,
  `showHideHiddenFiles`).

So the rebuild's panel is the same list of places plus an explicit Shortcuts
section around it, with the jump keys the person asked to see:

| what | value | where |
| --- | --- | --- |
| section | `Shortcuts`, foldable, `+` button | `shortcuts_header` in `app/src/main.rs` |
| rows | Home, Desktop, Documents, Downloads, Music, Pictures, Videos, Trash, then pinned folders in the order they were added | `App::shortcut_entries` |
| key badges | `Alt+1` … `Alt+9`, first nine rows, dim mono badge at the right edge | `row_impl` key argument |
| jump keys | `Alt+1` … `Alt+9` → `apply_tag("__sc_N__")` | `handle_shortcuts` |
| add | `+` in the header, the "Add Shortcut" row, the folder context menu, or `Ctrl+Shift+S`; the editor pre-fills the current folder | `Panel::shortcut_dialog` |
| rename / remove / reorder | right-click a pinned row: Open in New Tab, Rename Shortcut…, Move Up, Move Down, Shortcut Colour, Remove Shortcut; a row can also be dragged up or down | `places_panel` |
| colour | right-click any row — `Shortcut Colour` with the eight folder colours, or `Normal Colour` to clear it; the flag (or the standard row's icon) takes the colour | `theme::colour_menu`, `App::set_shortcut_color` |
| order | dragging a row into a gap moves it there; the gaps are counted like `KFilePlacesModel::movePlace` counts them, and a drop over the row's own place changes nothing | `config::move_shortcut` |
| duplicate rule | a folder that is already a standard row is refused with "That folder is already in the panel — click its row instead." | `App::save_shortcut` |
| persistence | the existing config file, `bookmarks` array plus `shortcuts_expanded`, `shortcut_keys` and `shortcut_colors` | `app/src/config.rs` |

Menu commands now print their key at the right edge (`menu_row`), the way the
original shows them: New Window `Ctrl+N`, New Tab `Ctrl+T`, New Folder `F10`,
Add Shortcut `Ctrl+Shift+S`, Find Duplicates `Ctrl+Shift+D`, Bulk Rename
`Ctrl+Shift+R`, Quit `Ctrl+Q`.

`Ctrl+N`, `Ctrl+Q`, `Alt+1…9`, `Ctrl+Shift+S`, `Ctrl+Shift+D` and
`Ctrl+Shift+R` are additions of this rebuild: the original's own key bindings
for new window and quit are KDE defaults but were not read out of this binary,
so they are marked "estimated" in `mechanics.json`. The badge look (dim mono
pill, 9 px, right edge) is a design choice measured against nothing in the
original, and is marked estimated too.

## Reordering and colouring the shortcuts (asked for by the person)

The person asked for two things: a colour on the shortcut flags, chosen the way a
folder's colour is, and dragging those flags into a different order.

**What the original does with an order.** Reordering lives in the Places view of
KIO, not in dolphin's own binary, and that library keeps its symbol table, so
these are real names and addresses in `libKF6KIOFileWidgets.so.6.31.0`. Ghidra
loads the library at image base `0x100000`, so a `nm` offset gains `0x100000` in
the decompiler.

| function | offset | decompiler address | what it does |
| --- | --- | --- | --- |
| `KFilePlacesModel::movePlace(int, int)` | 0x96220 | 0x196220 | the actual reorder (evidence `ev_e59ce745…`) |
| `KFilePlacesModel::dropMimeData(QMimeData const*, Qt::DropAction, int, int, QModelIndex const&)` | 0x92d00 | 0x192d00 | the drop entry point (evidence `ev_5fab7c5b…`) |
| `KFilePlacesView::dropEvent(QDropEvent*)` | 0xba880 | 0x1ba880 | turns the pointer position into a row, forwards an internal move (evidence `ev_75f4ae9f…`) |
| `KFilePlacesView::dragMoveEvent(QDragMoveEvent*)` | 0xae590 | 0x1ae590 | tracks the row under the pointer (evidence `ev_b12935eb…`) |
| `KFilePlacesView::paintEvent(QPaintEvent*)` | 0xadb80 | 0x1adb80 | draws the drop indicator (evidence `ev_9396d451…`) |

What `movePlace` says, read line by line:

* the second argument is an **insertion slot**, not a row: the function builds the
  gap before the bookmark at `param_2` and clamps it to the list length;
* `to == from` and `to == from + 1` are refusals — the code jumps to the same
  `uVar7 = 0` return in both cases, so letting a row go over its own place, or
  over the gap immediately below it, changes nothing;
* the destination is then pulled back to the dragged place's own **group** (the
  integer at `+0x118` of the item data, i.e. Places vs Devices vs Remote). The
  loops walk up or down until the group id changes and stop there, so a place can
  never be dragged into another section;
* the move itself is `QAbstractItemModel::beginMoveRows`, then
  `KBookmarkManager::root()` + `KBookmarkGroup::moveBookmark` on the bookmark's
  XML element, then the item array is shifted and `endMoveRows` is called; the
  function returns 1 when it moved;
* so the order the person sees is the order of the bookmarks in the bookmark
  file — there is no separate order number anywhere.

`dropMimeData` ignores a copy (a drop action of 0 returns 1 at once), accepts the
move only for column 0 with a real row and parent, and forwards the row to that
same move. `dropEvent` finds the index under the pointer, hands an internal drop
(`application/x-kfileplacesmodel`) to the model, and marks the event handled.

`paintEvent` is what the person sees while dragging: when the drop flag at
`+0x104` is set it draws a **line across the viewport in the gap** (pen taken from
the `Alternate` palette brush with its alpha lowered, several `QPainter::drawLines`
calls), and when the on-place flag at `+0x103` is set it highlights the row under
the pointer with a style primitive instead. So a drop between rows shows a line,
not a shifted row.

**What the rebuild does with that** (`app/src/config.rs`, `app/src/main.rs`):

* `config::move_shortcut(&mut Vec<Bookmark>, from, to)` implements exactly the
  slot rule above, including both refusals and the "down lands at `to - 1`" shift;
* the rows in the Shortcuts panel ask egui for a drag as well as a click, so a
  press-and-release in place is still an ordinary click (`row_tinted`);
* while a row is carried it is outlined and the pointer turns into a grabbing
  hand; the gap under the pointer gets a 2 px accent line with a dot at its left
  end, drawn from the row top when it is above a row and from the last row's
  bottom when it is past the end;
* dropping runs `move_shortcut` and says `Shortcut moved`; `Alt+1…9` follows the
  new order because the badges are read from the same list;
* `Move Up` / `Move Down` in the row's menu stay, so a person who would rather not
  drag can still reorder;
* only the person's own shortcuts move. The eight standard rows keep the order
  the original gives them, and a dropped standard row is refused, which is the
  same "same group only" clamp read out of `movePlace`.

**The colour.** The original has no coloured places at all: the only colour strings
in `/usr/bin/dolphin` belong to `KColorScheme` (theme handling), and no string
combines "place" with "colour". So this half is an addition.

* the palette is the folder-colour palette (`app/src/foldercolor.rs`, eight
  names and RGB values), and the menu itself is now one shared function,
  `theme::colour_menu`, used by both the folder menu in the file view and the
  shortcut rows, so the two cannot drift apart;
* the menu shows the colour in use with a 2 px white ring, and offers
  `Normal Colour` below the swatches, the same as the folder menu;
* a pinned shortcut's flag is filled with the colour (`PlaceIcon::Bookmark` is a
  filled polygon, so the whole flag takes it); a standard row's small icon takes
  it in the same way;
* the choice is stored in the rebuild's config under `shortcut_colors`, keyed by
  the folder the row points at, because a panel row has no file of its own to
  write to (a folder colour goes into that folder's `.directory`); picking
  `Normal Colour` removes the entry, and removing a shortcut removes its colour;
* a colour follows the shortcut, not the folder: the same folder shown twice
  (once as a standard row, once pinned) is coloured twice, deliberately, so a
  shortcut can look different from the folder it opens.

**Keeping the colour when the row is marked (reported by the person).** The
person said a shortcut "seems to lose its colour" when it is marked. It did: the
row painter drew a selected row's glyph in plain white —

    if selected { Color32::WHITE } else { tint.unwrap_or(PLACE) }

— so the moment a coloured row became the current folder the flag turned white,
and a row with no colour of its own lost its blue-grey too. The white came from
copying the original, where it costs nothing: measured in the original's own
window (`app/shots/orig-places.png`, fresh profile, Home row selected), every
Places icon is drawn at `#fcfcfc` whether selected or not, i.e. the original's
places are monochrome and cannot lose a colour when one is picked out. The
rebuild's places are tinted, so the same code threw information away.

The row now keeps its colour and only has its shade lifted, by
`theme::SELECTION_LIFT = 0.32` towards white, so it reads on the blue selection
band (`App::place_icon_colour`, used by `row_tinted`):

| row state | colour drawn |
| --- | --- |
| marked, colour `#c44842` (Red) | `#d7837e` |
| marked, no colour of its own | `#b8cbd8` (the place blue-grey, lifted) |
| unmarked | the colour exactly as chosen, or `#96b2c6` |

Measured in `app/shots/marked-colour.png` (61 px of `#d7837e` on the band) and
`app/shots/marked-plain.png` (60 px of `#b8cbd8`, no white glyph). `--selftest`
checks the rule: an unmarked row keeps its exact colour, a marked one stays red
rather than becoming white, and a row without a colour stays blue-grey.
`--keep-settings` was added so a picture can be taken with the person's own
shortcuts and colours instead of factory ones.

**Estimated (marked `estimated` in `mechanics.json`):**

* which of the two gaps a pointer is nearer to. `movePlace` only receives the
  chosen slot; the choice is made in Qt's item-view drop indicator, which this
  rebuild does not link. The rebuild compares the pointer's y with the row's
  centre line, the usual half-row rule;
* the width and alpha of the drop line: the original takes them from the theme's
  `Alternate` brush, and the exact values are not constants in the code read
  here. The rebuild uses its own accent colour at 2 px.


## The location bar: click to type, paste an address (asked for by the person)

The original's own help text for the location bar, string at `0x132138` in
`/usr/bin/dolphin` (referenced from `sub_65ec0`, the "what's this" text):

> "This toggles the *Location Bar* to be editable so you can directly enter a
> location you want to go to. You can also switch to editing by clicking to the
> right of the location and switch back by confirming the edited location."

So the rule read out of the program is:

| what happens | rule | evidence |
| --- | --- | --- |
| a click on the bar that misses the breadcrumbs | the bar becomes a text field | string `0x132138`, `sub_65ec0` |
| confirming the text | the address is opened and the bar goes back to breadcrumbs | same string ("switch back by confirming") |
| the toggle action | `Make location bar editable`, string `0x12e6db`, wired in `sub_d5050`; the shortcut is the KDE standard `Ctrl+L` | strings `0x12e6db`, `0x12e6f6` |
| the bar's text when opened | the current folder's path (`DolphinView::url()` / `viewPropertiesUrl()` at `0x173f90`, `0x17f0f0`) | named symbols in `libdolphinprivate.so.6` |
| a file address, not a folder | `DolphinView::setUrl` (`0x17e9b0`) only ever loads a directory, so the view is opened at the file's folder and then `DolphinView::selectFileOnceAvailable` (`0x173b40`) selects the item as soon as the listing's `itemsInserted` signal arrives | pseudo-C of `0x17e9b0` (`loadDirectory`), pseudo-C of `0x173b40` (connect to `itemsInserted`, then select) |
| where the field is | the toolbar row, in a rounded 1 px box with the breadcrumbs inside it | live capture `checks/orig-live/live.png` |

What the rebuild does with that (`app/src/views.rs`, `app/src/state.rs`):

* `Pane::breadcrumb_ui` registers the bar's whole rect as a click target. A
  click that lands on a breadcrumb navigates, as before; any other click inside
  the bar calls `Pane::begin_location_edit`.
* `begin_location_edit` fills the field with the current path and selects all of
  it, so `Ctrl+V` replaces the address instead of appending to it.
* `Pane::navigate_input` → `resolve_location` accepts what people actually
  paste: plain paths, `~`, `~/…`, `$HOME/…`, quoted paths, and `file://` links
  with `%20` escapes. A folder opens. A file opens its folder and
  `Pane::apply_pending_select` selects it once the listing is there, which is
  the `selectFileOnceAvailable` behaviour above. Anything else keeps the field
  open and says "There is nothing at "…"." instead of showing an empty folder.
* `Escape` cancels, `Enter` confirms, `Ctrl+L` toggles — the same three ways in
  and out as the original.
* The whole-address-selected-on-open detail is not in the strings we read; it is
  standard behaviour of the original's text field and is marked "estimated" in
  `mechanics.json`.

Checked by driving the built window on a virtual display: a click at the empty
right end of the bar turns it into a field with the path highlighted
(`app/shots/ui-click.png`), a pasted `…/gallery/readme.md` fills it
(`app/shots/ui-paste2.png`), and Enter returns to breadcrumbs with `readme.md`
selected and the status bar reading "1 item selected (10 B)"
(`app/shots/ui-enter.png`). The same rules are covered by `--selftest`:
"a pasted address for a folder opens it", "…for a file opens its folder and
selects the file", "a file:// link and a home shortcut both resolve",
"an address that leads nowhere says so and stays put".

Two things that came out of testing the bar in a running window and are now fixed
in the code:

* A start is **one view**. Split is a choice made in the window (F3, the menu
  switch, or *In the other pane*) and is neither restored nor written back to the
  settings file, so the app never opens as two folders and never asks the person
  to undo a choice they did not make this run. An older file that says
  `split: true` opens single, and the next save clears the flag.
* A captured picture used to write the factory config back over the person's
  settings (pinned folders included). Saving is now skipped in `--screenshot`
  mode, which also keeps the check image identical from run to run.

Verified again in the running window at the time of this build: click on the
empty end of the bar → field with the path selected; `Ctrl+V` with a path on the
clipboard → `/home/mjbayes/Pictures` in the field; Enter → the folder opens with
breadcrumbs back and the other pane untouched.

**A path with `..` in it no longer leaves a phantom segment (fixed this round).**
Found by opening the rebuild with `--dir …/app/../checks/demo`: the breadcrumb is
built from the path's plain components, so it drew `… dolphin > app > checks >
demo` — the `app` that the address steps out of, with the `..` itself silently
dropped, because the breadcrumb only keeps `Component::Normal` parts. The
original never shows this: its location bar is built from the folder it is in
(`DolphinView::url()` / `viewPropertiesUrl()` at `0x173f90` / `0x17f0f0`, and the
KIO breadcrumb below them), not from the string that led there, so a folder has
exactly one breadcrumb trail however it was reached.

`fsutil::tidy_path` now resolves `.` and `..` lexically before the path is
stored, from `Pane::load` — the funnel every navigation already goes through
(places, tabs, address bar, Back/Forward, split view). Rules, as written:

| input | shown |
| --- | --- |
| `/usr/share/../lib` | `/usr/lib` |
| `/home//me/./notes/` | `/home/me/notes` |
| `/..` | `/` |
| `trash:/` (a virtual location, not a folder on disk) | unchanged |
| a relative path | unchanged |

It is lexical on purpose: resolving it with the filesystem would turn a symlink
into its target and change where Back and the breadcrumbs point. `--selftest`
covers all five rows above ("the location bar shows the folder itself, not a
path with dots in it"). Verified in the running window: started with
`--dir …/app/../checks/demo`, the bar reads
`/ home mjbayes Documents Craft Anything dolphin checks demo`
(`app/shots/tidy-crumb.png`), and the Add Shortcut dialog's pre-filled folder is
likewise the tidy path.


## The icon grid: how much space sits between columns (asked for by the person)

The person asked for the icons view to stop wasting horizontal space so more
icons fit on screen at once. Two kinds of evidence went into the change.

**The original, measured.** Two captures of dolphin 26.08.2 in icons view, same
window and same 592 px wide view, zoomed to 64 px and to 160 px
(`checks/orig-zoom/icons-64px.png`, `checks/orig-zoom/icons-160px.png`), were
measured pixel by pixel: five demo files lay out as 4 columns at 64 px with the
icon centres **148 px** apart, and as 3 columns at 160 px with centres **197 px**
apart. In both cases the pitch is the view width divided by the column count
(592 / 4 = 148, 592 / 3 = 197), i.e. the original distributes the free width
between the columns instead of leaving it all at the right edge.

**The original, in code.** `KItemListViewLayouter::doLayout()` @0x161bb0 does the
same arithmetic: it divides the usable view width by the item width
(`vdivsd` @0x161c79), truncates (`vcvttsd2si` @0x161c8a) and clamps the result to
at least one column (`cmovle` to 1 @0x161c90). The per-item width itself comes
from the size-hint resolver (`KItemListSizeHintResolver::sizeHint` @0x159820,
filled by `updateCache` @0x1597c0 → `KItemListView::calculateItemSizeHints`
@0x103120); how that hint mixes the icon size and the label width is not
traced, so the *effective* column counts above are quoted from the captures
rather than derived from the hint formula.

**What the rebuild does now** (`app/src/views.rs`, `views::icon_grid`):

* a cell needs `max(icon + 24, 88)` px — the width now follows the zoom instead
  of sitting on one fixed floor, which was the old bug: the previous code used
  `max(icon * 1.35, 96) + 18`, i.e. 114 px for every icon size from 16 to 64 px,
  so at 32 px icons the gap between columns was wider than two icons;
* the columns share the free width evenly (`pitch = width / columns`) with a 2 px
  gap, as the original does, so nothing is stranded at the right edge;
* the cell is `icon + 46` px tall when the label needs two lines and `icon + 32`
  when it needs one (below 32 px), because the old height clipped the second
  line at small zooms;
* labels now break after a space, hyphen, underscore, dot or slash and fall back
  to a hard split, so "archive-backup.zip" fills two lines instead of being cut
  to "archive-back…" (`views::wrap_label`).

Result at a 1000 px wide view: 11 icons per row at 64 px and 11 at 32 px, against
8 and 10 before. The original fits 4 per 592 px at 64 px (≈7 per 1000 px), so the
rebuild is deliberately tighter than the original here — that is what the person
asked for. `--selftest` guards it with "the icon grid packs many files into a
row", and `mechanics.json` carries the numbers under the `icons view` group.


## Portrait pictures: the cell follows the picture, not the square (asked for by the person)

The person said that a portrait picture still reserves a square slot, so the
space left and right of it is wasted. That is exactly what the original does,
and now the rebuild does not.

**The original, in code.** `KItemListViewLayouter::doLayout()` in
`libdolphinprivate.so.26.08.2` at **0x261bb0** (the same function sits at
0x161bb0 in `/usr/bin/dolphin`) works in two steps:

* the number of columns is `floor(view width / (item margin + item width))`,
  clamped to at least one (`vdivsd` at 0x261c79, `cvttsd2si` at 0x261c8a,
  clamp at 0x261c90), and then the leftover width is shared equally between the
  columns (`0x261cd0`–`0x261ce0`: `leftover / (columns + 1)` added to the pitch);
* every item in a row is given the *same* width: the size-hint width is raised
  to at least the configured item width (`vmaxsd` at 0x261ffb, stored once per
  column), and the row breaks after exactly `columns` items.

So the column is as wide as the square icon box no matter what is drawn in it.
Inside that box `KStandardItemListWidget::drawPixmap` @0x135bb0 fits the
preview with `KPixmapModifier::scale` @0x15e820: aspect kept, picture centred.
A 3:4 photo therefore fills 48 of a 64 px box and a 9:16 photo only 36, with the
rest empty on both sides — the person's complaint, confirmed in code.

**What the rebuild does now** (`app/src/views.rs`):

* `views::thumb_size(icon, aspect)` keeps the same aspect-fit rule as the
  original, so pictures are drawn at exactly the size the original would give
  them;
* `views::icon_cell_width(icon, aspect)` makes the *cell* that wide instead of
  square, plus 4 px of padding on each side (`ICON_CELL_PAD`). A cell never
  drops below `views::label_floor` = `clamp(icon × 0.75, 40, 96)` px, which is
  what keeps a two-line name from being squeezed to nothing;
* because cells now differ in width, the fixed column count is replaced by
  `views::pack_icon_rows`: a shelf layout that walks left to right and starts a
  new row when the next cell no longer fits. Row height stays `icon + 46` px
  (two label lines) or `icon + 32` px (one), so labels still share a baseline;
* the picture shape comes from `search::probe_image_size`, which reads the
  width and height out of the JPEG/PNG/GIF/BMP/WebP header once per file and
  caches it — no decoding, so a 300-photo folder still lays out instantly. A
  file whose header cannot be read is treated as square;
* a wide picture keeps the full cell width but is drawn shorter and centred
  vertically in the box, since its empty space is above and below rather than
  at the sides;
* when the list is long enough to scroll, the rows are packed a second time with
  the scroll bar's ~14 px strip taken off the width, so the last cell of a row is
  never pushed under the bar;
* a name that would leave one or two letters alone on the second line keeps a
  single, fuller line instead ("pic-000.jpg" stays on one line rather than
  breaking to "pic-000.jp" + "g").

Effect at 64 px icons and a 1000 px wide view: a square picture or a folder
still gets a 72 px cell (13 per row), while a 3:4 photo gets 56 px and a 9:16
photo 56 px as well (17 per row), against 72 px and 13 per row before this
change and 100 px in the version before that. In `~/Pictures` with phone
screenshots the first row went from 11 items to 15.

`--selftest` guards the rule with "a tall picture gets a tall, narrow cell"
(9:16 picture measured at 36×64 px, cell narrower than the square one, at least
17 per 1000 px row) and keeps the older "the icon grid packs many files into a
row" check. `mechanics.json` carries the numbers under the `icons view` group.

**Not done:** the cell width follows the picture only; the *row* height is still
one value for all items, so a very wide picture leaves vertical space above and
below itself. A justified gallery layout (rows stretched to fill the width)
would use that space too, but it would break the even label baseline this view
keeps with the details and compact views.

## The picture wall: rows that fill the width (asked for by the person)

The person liked the new *Pictures — thumbnails of this folder* button but said
the thumbnails sit on a square footprint, which leaves wasted space between
them. That square is the one the wall inherited from the image browser it was
rebuilt from: a card is `width: var(--thumb-size)`, and the picture is fitted
inside it. At 200 px a 16:9 shot is therefore drawn 200×112 — the card is 88 px
taller than the picture — and a 9:16 shot is drawn 112×200, so the card is 88 px
wider than the picture. Two portrait pictures consequently stood 44 + 12 + 44 =
**100 px** of empty wall apart, and two wide ones had the same 100 px between
their rows; the square card, not the picture, decided the spacing. Even two
square pictures were 11 + 12 + 11 = **34 px** apart, because each card carried
8 px of padding and a 3 px mat around a 194 px picture. The wall now leaves
exactly the 12 px gap between any two pictures, square or not.

Seen in the previous build's own window picture (`app/shots/gallery1.png`,
200 px cards): five cards in a row, five square wells, each holding a picture
with a band of empty wall above and below it or left and right of it.

**What the wall does now** (`app/src/gallery.rs`):

* every picture is laid out at its own aspect ratio; the frame around a
  thumbnail is the picture and nothing else, so the browser's 8 px card padding
  is gone (the 12 px gap between pictures is kept, `wall_rows`);
* a row takes pictures until the next one would no longer fit beside them at the
  slider's height, so every picture on the wall is exactly that tall (**changed
  in the pass described under "The picture wall keeps one size for every row"
  below — the earlier version stretched each row to the window width instead,
  which is what made the last row look bigger than the rest**);
* the slider now means *height of the pictures in a row* rather than the side of
  a square, still 50–400 px;
* the last row is packed like every other one, so it carries the same height as
  the rows above it; `ROW_STRETCH_MAX` (1.6 × the slider height), which used to
  cap a lone leftover photo, is gone with the stretching;
* the aspect ratio is held inside 1:5 … 6:1 (`ASPECT_MIN` / `ASPECT_MAX`) so a
  damaged header or a stitched panorama cannot take a whole row; folders and
  pictures whose header cannot be read are laid out square, as before;
* the packing is virtualised by the scroll area's clip rectangle: rows above or
  below the visible band are skipped, so a folder of a thousand pictures still
  only draws the dozen on screen.

At 200 px in a 1180 px pane the demo folder of 13 entries laid out **5 + 5 + 3**
across three rows, with no empty band inside any row (`app/shots/wall/mixed200.png`);
at 400 px the same folder gives rows of 3 (`mixed400.png`), at 50 px all 13 fit
in one row (`mixed50.png`). A folder of three pictures (`pair.png`) puts them
side by side at 1.6 × the asked height, flush left, instead of three square
wells with a picture in the middle of each.

`--selftest` guards the rule with "every picture is as wide as its own shape, so
no bands sit between them": it packs a wide, a tall, a square, a 4:3 and a 3:2
picture into a 1000 px wall, then checks that every row marked full adds up to
1000 px and that the wide picture is at least twice as wide as the tall one.
`mechanics.json` carries the numbers under the `picture wall` group.

**Left alone on purpose.** Only the added picture wall packs whole rows. The
icons view keeps its own shelf layout from the earlier change — a cell as wide
as its picture needs, with one height for the row — so a very wide picture there
still leaves a little wall above and below itself; the compact and details views
stay exactly as the original draws them. Stretching those rows too would break
the label baseline the icons view shares with the details view, which is the
trade-off noted above.

## Read out of the code with the decompiler: the status bar, the selection, the disk-space button

Everything in this section comes from `native_function_pseudocode` (Ghidra
12.1.4) on the two installed files — `/usr/bin/dolphin` (image base `0x100000`,
6865 procedures) and `/usr/lib/libdolphinprivate.so.6` (26.08.2, image addresses
equal to file offsets). The raw answers are kept in `rea/code/` and `rea/mcp2/`;
the shell script that asks for them is `rea/batch.py`.

### Status bar text — `DolphinView::emitStatusBarText` @0x273180

The function takes `(int folderCount, int fileCount, unsigned long long
fileSize, DolphinView::Selection)` and builds the line from several plural
message ids instead of one string. What it does, in order:

| step | rule | string |
| --- | --- | --- |
| folder phrase | `folderCount == 1` → singular, else `%1 folders` | `1 folder` `0x1bf6ab`, `%1 folders` `0x1bf6a0` |
| file phrase | `fileCount == 1` → singular, else `%1 files` | `1 file` `0x1bf6bd`, `%1 files` `0x1bf6b4` |
| nothing at all | `fileCount < 1 && folderCount < 1` → the empty line, no size | `0 folders, 0 files` `0x1bf6ea` |
| folders only | `fileCount < 1` → the folder phrase **without any size** | — |
| files only | `folderCount < 1` → `"%1 (%2)"` with the file phrase and the size | `%1 (%2)` `0x1becea` |
| folders and files | otherwise → `"%1, %2 (%3)"` | `%1, %2 (%3)` `0x1bf6c4` |
| size | `KFormat::formatByteSize((double)fileSize, 1, …)` — one decimal, binary units | — |

So the earlier note in this sheet that quoted a single `%1 folders, %2 files (%3)`
string is wrong: no such string exists in either binary. The rebuild's
`status_counts` (`app/src/main.rs`) now follows the table above; `--selftest`
checks the four shapes.

### The same line when something is selected — same function, `Selection` flag

With the selection flag set (`param_5 == 0`) the very same composition runs, but
both phrases switch to their "selected" plural forms:

* `1 folder selected` `0x1bf66c`, `%1 folders selected` `0x1bf658`
* `1 file selected` `0x1bf690`, `%1 files selected` `0x1bf67e`

The consequences, all of which the rebuild now reproduces:

* a selection of files carries the size of the selected files —
  `2 files selected (1.5 KiB)`; the old code dropped the size;
* folders alone never carry a size — `1 folder selected`;
* a mixed selection is *not* collapsed to "%1 items selected":
  it reads `1 folder selected, 2 files selected (1.5 KiB)`;
* `%1 items selected` (`0x22ed75` in `/usr/bin/dolphin`) is used by the
  **Information panel**, not by the status bar (next but one section).

`--selftest` now checks all three shapes.

### The disk-space button — `FUN_001e6a40` @0x1e6a40 in `/usr/bin/dolphin`

This is the function that fills the status bar's free-space indicator. It reads
the total size at `+0x20` and the free size at `+0x28` of the space info object
and computes the used share as

     pct = (int)(((double)(total - free) * 100.0) / (double)total + 0.5)

i.e. **round half up** before truncating. A volume with 999 of 1000 bytes used
therefore shows `100%`, where the rebuild's old integer division
(`(t - f) * 100 / t`) showed `99%`. `fsutil::used_percent` carries the formula
and `--selftest` pins it (`999/1000 → 100%`, `510/1010 → 50%`).

The same function also settles what the bar *says*:

| element | text | string address |
| --- | --- | --- |
| button text | `%1 free` — `KIO::convertSize(free)` only | `0x12ec43` |
| tooltip | `%1 free out of %2 (%3% used)` | `0x12ec68`, and `0x137448` with the extra line `Press to manage disk space usage.` (`0x137465`) |

When the volume reports no size (`total == 0`) the widget is hidden outright
(`QWidget::hide`), so nothing is printed. The rebuild now shows `355.6 GiB free`
in the bar with the full sentence in the hover text and nothing at all when the
total is unknown; the "Press to manage disk space usage" second line is left out
because this rebuild has no disk-usage window to open.

### Information panel with several items selected — `FUN_001f76f0` @0x1f76f0

The selection count is read at `+0x98`. When it is **greater than one** the
function builds a `dialog-information` icon (`QIcon::fromTheme`), sets the label
`%1 item selected` (plural form `%1 items selected`, string `0x22ed75`), clears
the per-file rows (`Baloo::FileMetaDataWidget::setItems`) and hides the widget —
so a multi-selection shows the information icon and the count, **without a
total size**. With exactly one item it shows that item's metadata instead, and
with none it falls back to `KIO::stat` of the folder URL.

The rebuild's Information panel used to append the size in brackets to the
count; it now shows the circled information mark and `N items selected`, and the
size of a selection lives only in the status bar, where the original puts it.

### Factory defaults — the three settings constructors

Read out of the KConfigSkeleton `ItemInt`/`ItemBool` calls (the last argument is
the default), not from the person's own `dolphinrc`:

| settings object | address | factory values |
| --- | --- | --- |
| `IconsModeSettings::IconsModeSettings` | `0x2b1c80` | `IconSize` 32, `PreviewSize` 64, `TextWidthIndex` 1, `MaximumTextLines` 3 |
| `CompactModeSettings::CompactModeSettings` | `0x2af830` | `IconSize` 16, `PreviewSize` 48, `MaximumTextWidthIndex` 0 |
| `DetailsModeSettings::DetailsModeSettings` | `0x2b10d0` | `IconSize` 16 (at `+0x2c`), `PreviewSize` 48 (at `+0x30`), `LeftPadding` 20, `RightPadding` 20, `HighlightEntireRow` true, `ExpandableFolders` true |

`DolphinView::updateDefaultZoomLevel` @0x27df20 then converts those sizes into
the zoom a fresh view opens with: it takes `PreviewSize` while previews are
shown and `IconSize` when they are off, and calls
`ZoomLevelInfo::zoomLevelForIconSize` @0x2a6560. That gives **icons 4, compact 3,
details 3** with previews (64, 48, 48 px) and **icons 2, compact 0, details 0**
without (32, 16, 16 px). The earlier "estimated" defaults in this sheet are
replaced by these numbers, and a config file that predates the `zoom` key now
falls back to the same 4/3/3 instead of 4/0/0.

### Ctrl + wheel — `DolphinView::wheelEvent` @0x272710

Kept here because the person asked for wheel zoom: the handler first tests the
modifier byte (`param_1[0x23] & 4`, i.e. Qt's ControlModifier); without it the
event is accepted and nothing happens. With it, the wheel delta (Qt's
1/8-degree units) is added to a running sum at `+0xcc`; every `0x78` = **120**
units — one wheel notch — calls `slotIncreaseZoom` (or `slotDecreaseZoom`) once
and subtracts 120 from the sum, keeping the remainder for the next event.
`--selftest` and the rebuild's `state::wheel_zoom_steps` implement exactly that.

### The toolbar row: the location bar and the buttons beside it

The person reported that the sort menu overlapped the address bar. Two separate
faults produced that, and both are in the rebuild's own layout, not in the
original's:

1. **A guessed width for the right-hand cluster.** `App::top_toolbar` reserved a
   fixed 268 px for the buttons to the right of the location bar — menu, search,
   filter, split, the three view buttons and the sort control — while those
   controls actually need 317–327 px at the rebuild's metrics (7 × 30 px icon
   buttons, two 4 px separators, nine 2 px gaps, plus the sort button's measured
   text). `Layout::right_to_left` then started 50–60 px further left than the
   space it had been given and painted the sort label on top of the bar. At
   1440 px the overlap fell into the bar's empty right end; at 980 px it landed
   on the bar's border and text (`app/shots/toolbar-before.png`).
2. **A bar that grew to fit its contents.** The bar was an `egui::Frame` whose
   inner `Ui` got `set_min_width`; the breadcrumb buttons are unbreakable, so a
   long path made the frame's own rect wider than the row allotted to it and its
   background was painted underneath the sort control. The field's outline now
   has a fixed rect, its contents are clipped to it, and the breadcrumbs elide.

What the original does, from its own window (`checks/sort-bar/orig-760.png`):
the toolbar is Back / Forward / Up, then a rounded location box, then the
view-mode and view buttons; the box **ends before the buttons** and is the part
that gives way when the window narrows. That is the rule the rebuild now keeps.

| what | value | where it comes from |
| --- | --- | --- |
| toolbar row height | 46 px | live capture `checks/sort-bar/orig-760.png` |
| toolbar icon control | 30 × 30 px | `app/src/main.rs`, `toolbar_button` |
| gap between bar and cluster | 8 px | rebuild spacing, **estimated** |
| narrowest location bar worth drawing | 110 px | rebuild rule, **estimated** |
| sort control, wide window | label `Sort: Name` | rebuild |
| sort control, narrow window | icon only | rebuild |
| sort control, narrowest | hidden; sorting stays in the app menu's *Sort by* | rebuild |
| the two words (`Find`, `Menu`) | kept to the last; they go only when even a worded row with no sort button will not fit (about 460 px of window) | rebuild rule, changed this round: a row of bare glyphs is what the person could not read |
| long paths | segments drop from the left behind `…`; the current folder stays | rebuild, **estimated** (the original's elision lives in the KIO breadcrumb code) |

`toolbar_metrics(avail, label_w, show_bar)` in `app/src/main.rs` now owns that
arithmetic and keeps one invariant: `bar_w + 8 + cluster_w <= avail` whenever a
bar is drawn. `--selftest` checks it for every window width from 420 to 1800 px
and for four sort-label widths, and also checks that split view reserves no bar
at all.

Verified in the running window on a virtual display: with `FILE_CRAFT_TOOLBAR_DEBUG=1`
the app prints the real rects — at 980 px wide `avail=791`, bar 466.2 px ending at
x=647, cluster 316.8 px starting at x=663, sort button 663–740, no overlap; at
700 px `avail=511`, bar 186.2, sort 383–460. Screenshots at 1440, 980, 820, 700
and 620 px (`app/shots/toolbar-*.png`) show the label clear of the bar, then the
icon-only sort at 620 px, and the menu opening below the button, entirely under
the toolbar. Split view (F3) keeps its per-pane location bars and the sort
control in the toolbar with no bar to collide with. In a window that is not
maximised the app menu is capped to the window's own height and scrolls (its bar
always drawn), because the rows below the fold — Help, About, Quit — were once
simply unreachable: the popup ran off the bottom of the screen and could not move.

### The buttons at the right end: what each one owns

The person sent a 163 x 60 crop of three unlabeled glyphs — a small triangle, a
magnifier and three lines — and asked what each one uniquely should do. The
triangle and its list have since been removed at his request, so the row now
carries the magnifier and the three lines. The
original's own strings and its settings constructor say what the trio is there:

| glyph | the original's string | what it does there | evidence |
| --- | --- | --- | --- |
| magnifier | `Toggle Search Bar` | opens a **search bar**: the tooltip promises to "find files and folders", and Enter looks below the folder | string at file offset `0x12d7de`, action built at `0x67fc8` |
| small triangle | `Open in New Tab` / `Open in New Window` / `Open in Split View` | a split button's destination list — where the picked item opens; the rebuild dropped that toolbar control altogether and keeps the destinations on the folder's right-click menu | strings at `0x12da05`, `0x12da26`, `0x12da39`, built at `0x6bf86`, `0x6c286`, `0x6c40a` |
| three lines | the application menu | the app's own switches, tools and settings | the toolbar's control button |

The confusion is the original's, and it is earned: the same window also carries
a **filter** bar, a different control with different wording — `FilterBar` is a
`GeneralSettings` bool at `this+0x82`, default false, added to kconfig in
`GeneralSettings::GeneralSettings` (string `FilterBar` @`0x002c0a90`, referenced
at `0x2b8853` and `0x2b8886`), with `Remove Filter` (string `0x12e119`, used at
`0xbcaef`) and `Keep Filter When Changing Folders` (`0x1350c8`). So "narrow
what I am looking at" and "search below this folder" are two controls in the
original, and the magnifier is the second one.

The rebuild gives each button **one job and the word for it** — two buttons, because
the person asked for the toolbar's `Open` (formerly `Open in` with its arrow) to go
as well, and opening a file or folder is what a double-click and the right-click
menu already do:

| button | word | the one job | what it must not do |
| --- | --- | --- | --- |
| Find | `Find` | narrows the list in place as you type; two chips, *This folder* / *In subfolders*, say how far the looking goes | never opens anything, never changes a setting |
| Menu | `Menu` | the app's own settings and tools, grouped New / Tools / View / Help | never touches the person's files |

Three faults were fixed to make that true:

1. **The Menu button was dead to an ordinary click.** It was wired to a
   right-click popup (`Response::context_menu`), so a left click closed a menu
   that had never opened. It is now `Popup::menu`, which toggles on a plain
   click; `--menu` opens it on the first frame so a picture can be taken.
2. **The find bar changed the words and not the list.** The startup path (and
   any path that set the words without a keystroke) never narrowed the listing;
   `Pane::refilter` is now called there too, and `--selftest` types "ap" into a
   four-item folder and asserts the three matching names, the selection count,
   and that closing the bar restores all four.
3. **The words gave way before the sort button.** The toolbar ladder now drops
   the sort key's name first, then the sort button (sorting stays in the
   menu), and only then the words — measured against the location bar's own
   110 px floor at every width from 360 to 1600 px.

The card that spells the three jobs out was unreachable for the same class of
reason: `guide_open` was never set by `--guide`, so the dialog existed and never
drew. It now opens from the menu, from F1, and from `--guide`
(`app/shots/now/guide2.png`).

## The deep pass: the main program read function by function

This round the original's **main program** `/usr/bin/dolphin` (26.08.2, stripped,
sha256 `341e417b…fad937`) was opened as the analysis target and read with the
decompiler, not with `strings` alone. REA imported it at image base `0x100000`,
so **a Ghidra address is the file offset plus `0x100000`** (a function at file
offset `0xe5750` is quoted as `0x1e5750`). The same base applies to
`libdolphinprivate.so.26.08.2`, whose dynamic symbols are real names.

Strings were found first, then their references were followed into functions:

| string | file offset (= analysed address − 0x100000) | read from |
| --- | --- | --- |
| `Natural` | 0x12eaf2 | `FUN_001e5750` |
| `Alphabetical, case insensitive` | 0x1372e0 | `FUN_001e5750` |
| `Alphabetical, case sensitive` | 0x12eafa | `FUN_001e5750` |
| `%1 items selected` | 0x12ed75 | `FUN_001f76f0` |
| `%1 free` | 0x12ec43 | `FUN_001e6a40` |
| `%1 free out of %2 (%3% used)` | 0x12ec68 | `FUN_001e6a40` |
| `Preview icon size:` | 0x12e777 | `FUN_001dcdb0` |
| `Show previews for` / `… for folders` | 0x12ef59 / 0x12efa6 | `FUN_00217c30` |

Functions read (pseudo-code kept under `rea/deep/`, whole-batch evidence
`ev_4d98f818…b70f97`, settings constructor `ev_e6212217…87f678eb`):

| function | address | what it is |
| --- | --- | --- |
| `FUN_001e5750` | 0x1e5750 | the **View → Display** page of the settings dialog (sorting mode, folder size, date style, permissions style, long names) |
| `FUN_001dcdb0` | 0x1dcdb0 | that dialog's view page: icon/preview size sliders, font, maximum width, details rows |
| `FUN_001f76f0` | 0x1f76f0 | Information panel update (selection count, icon, label) |
| `FUN_001e6a40` | 0x1e6a40 | status bar free-space widget (text, tooltip, capacity bar, hide) |
| `GeneralSettings::GeneralSettings` | 0x2b8630 (main program) | the stored sorting choice and its factory default |

### The three name comparisons, and which one is the default

`FUN_001e5750` builds three `QRadioButton`s in this order, each label passed to
`ki18ndc` with the dolphin context:

1. `Natural` — 0x12eaf2, stored at `this+0x28`
2. `Alphabetical, case insensitive` — 0x1372e0, stored at `this+0x38`
3. `Alphabetical, case sensitive` — 0x12eafa, stored at `this+0x30`

(the third is constructed last but kept at the lower offset, and
`QButtonGroup::addButton` is called in the order +0x28, +0x38, +0x30 — the row
order above is the order they are laid out in).

The value they set is `GeneralSettings::SortingChoice`, an `ItemEnum` created in
`GeneralSettings::GeneralSettings` @0x2b8630. Its three enum entries are built in
this order — `NaturalSorting`, `CaseInsensitiveSorting`, `CaseSensitiveSorting` —
and the `ItemEnum` constructor's last argument is `0`, i.e. **a fresh profile
sorts by `NaturalSorting`**. The three names are the config keys under
`[General]`, so a saved choice is `SortingChoice=NaturalSorting` etc.

What the rebuild now does: `state::NameSort` (Natural / CaseInsensitive /
CaseSensitive) with the same 0/1/2 order, `config.name_sort` written as
`NameSort::index()`, and the three rows in a *Name comparison* submenu of the
sort control **and** of the app menu's *Sort by*, carrying the original's exact
wording. `--selftest` checks the ordering each mode produces
(`file2 < file10`, `B.txt < a.txt`, `a.txt < B.txt`) and that the saved index
round-trips. Pictures: `app/shots/deep/11-sortmenu.png`, `12-submenu.png`.

### The rest of the page those radios sit on — `FUN_001e5750` @0x1e5750

The sorting radios are not a sort dialog of their own. The same widget builds
every row of the **View → Display** settings page, in this order, each label
followed by its `@title:group` heading:

| row | strings (analysed addresses in `/usr/bin/dolphin`) | what it sets |
| --- | --- | --- |
| sorting mode | `Sorting mode: ` 0x22eb17 → `Natural` 0x22eaf2, `Alphabetical, case insensitive` 0x2372e0, `Alphabetical, case sensitive` 0x22eafa | `GeneralSettings` key `SortingChoice` |
| folder size | `Folder size:` 0x22eb66 → `Show number of items` 0x22eb26, `Show size of contents, up to ` 0x22eb3b, `Show no size` 0x22eb59, plus a `QSpinBox` beside the middle row (`setRange(1, …)`, step 1) | `ContentDisplaySettings` keys `DirectorySizeMode` (0/1/2 in that row order) and `RecursiveDirectorySizeLimit` (the spin box) |
| date style | `Date style:` 0x22eb9d → `Relative (e.g. '%1')` 0x22eb73, `Absolute (e.g. '%1')` 0x22eb88 | `ContentDisplaySettings` key `UseShortRelativeDates` (true = relative) |
| permissions | `Permissions style:` 0x22ec01 → `Symbolic (e.g. 'drwxr-xr-x')`, `Numeric (Octal) (e.g. '755')`, `Combined (e.g. 'drwxr-xr-x (755)')` | `UsePermissionsFormat` (bool) with the three format strings `SymbolicFormat` / `NumericFormat` / `CombinedFormat`, all in the ctor below |
| long names | `Long file names:` 0x22ec32 → one radio labelled `Long file names` 0x22ec14 | not traced to a key: the page toggles a single radio, and the ctor's `ElidingMode` (`Right` / `Middle`) belongs to a different group |

The values those rows write are read out of `FUN_001e7af0` @0x1e7af0, the page's
save routine: it tests the three folder-size radios in turn and stores
`DirectorySizeMode = 0, 1, 2` (`0` = number of items, `1` = size of contents,
`2` = no size), stores the spin box in `RecursiveDirectorySizeLimit` and the
sorting radios as `SortingChoice = 0, 1, 2`. Every write is guarded by
`KCoreConfigSkeleton::isImmutable`, so a system-wide lock file wins over the
dialog — the same reason the rebuild's saved settings are only defaults.

The factory values live in the two settings constructors, both in the **main
program**, group `[General]` / `[ContentDisplay]` of `dolphinrc`:

| constructor | address | item | default |
| --- | --- | --- | --- |
| `GeneralSettings::GeneralSettings` | 0x2b8630 | `SortingChoice` (`ItemEnum`, at `this+0xac`) | 0 — `NaturalSorting` |
| | | `HideXTrashFile` (`ItemBool`) | false |
| `ContentDisplaySettings::ContentDisplaySettings` | 0x2b2b70 | `DirectorySizeMode` (`ItemEnum`, at `this+0x18`) | 0 — `ContentCount`, i.e. **a folder shows how many items it holds, not a size** |
| | | `RecursiveDirectorySizeLimit` (`ItemUInt`, at `this+0x1c`) | **10** |
| | | `UseShortRelativeDates` (`ItemBool`, at `this+0x20`) | **true** |

Evidence: `FUN_001e5750` references `ev_e22ccbc6…b84f89b` (string references),
`FUN_001e7af0` `ev_092db445…5202a7b`, `ContentDisplaySettings` ctor
`ev_2ee237b2…be05139a`, `GeneralSettings` ctor `ev_e6212217…87f678eb`; all from
REA's Ghidra 12.1.4 session on `/usr/bin/dolphin` (sha256 `341e417b…fad937`).

So the rebuild's folder rows already show the original's factory choice: a count
(`12 items`), never a recursive size, and the size only after the person asks for
it. The one behaviour this pass changed is the tie-break: in the original one
`SortingChoice` decides how two names compare wherever a name is the deciding
key, so sorting by size, type or date now breaks ties with
`NameSort::compare` rather than always with the natural comparison
(`state::sort_entries`). `--selftest` pins it: with
`Alphabetical, case sensitive` two same-size files order `B.txt, a.txt`, and with
`Natural` they order `a.txt, B.txt`.

### The free-space widget is a button, a bar and a tooltip — `FUN_001e6a40` @0x1e6a40

Read line by line, in this order:

* it returns early — `QWidget::hide()` — when the volume reports no total size
  (`total == 0`) or the widget is flagged invisible, so nothing is printed at all;
* `used% = (int)(((double)(total - free) * 100.0) / (double)total + 0.5)` — round
  half up, **then** truncate (the rebuild's `fsutil::used_percent`);
* the button text is the message id at 0x12ec43 (`%1 free`) with
  `KIO::convertSize(free)` — the free bytes only;
* the tooltip is 0x12ec68 (`%1 free out of %2 (%3% used)`) filled with free,
  total and that same rounded percentage;
* **and the widget also drives a `KCapacityBar`**: `KCapacityBar::setValue(iVar4)`
  with the identical percentage, i.e. a bar filled to the used share, next to the
  button. It is set inside the same `setUpdatesEnabled(false/true)` pair as the
  text, so bar and number always agree.

The rebuild had the text and the tooltip but no bar. It now draws one, 34 × 6 px,
filled to `fsutil::used_percent(free, total)` — visible in the status bar of
`app/shots/deep/10-window.png` between the `… GiB free` label and the `NN px`
zoom label. The tooltip still reads `%1 free out of %2 (%3% used)`.

### The Information panel with more than one item — `FUN_001f76f0` @0x1f76f0

The count is at `+0x98`. When it is **greater than one** the function:

* stops the preview job and any preview movie for the previously shown item;
* loads the `dialog-information` icon from the theme and puts it in the preview
  label (`QIcon::fromTheme("dialog-information")`, then `QIcon::pixmap`);
* sets the label from `ki18ndcp(..., "@label", "%1 item selected")` with the count
  as `%1` — the plural form is `%1 items selected` (string 0x12ed75);
* calls `Baloo::FileMetaDataWidget::setItems()` with an empty list and then
  `QWidget::hide()`, so **no per-file rows and no total size** are shown.

With exactly one item the item's metadata is shown; with none it falls back to
`KIO::stat` of the folder URL. The rebuild already behaved this way (circled
information mark + `N items selected`, size only in the status bar) — this pass
confirms it against the code instead of the capture.

### The view page behind the sort page — `FUN_001dcdb0` @0x1dcdb0

Same dialog, per view mode (`param_2`: 0 icons, 1 compact, 2 details). What it
builds, which is what the original lets a person set:

* two `QSlider`s — icon size and preview size — plus the font row (a combo with
  *system font* / *custom font* and a `Choose font…` button);
* icons mode: a `Maximum width:` combo whose entries are inserted in this order —
  `Unlimited`, `Small`, `Medium`, `Large` (message id `@item:inlistbox Maximum
  width`) — this is the `TextWidthIndex` the icon-cell formula uses;
* details mode: a `Folders:` row with an `Expandable` checkbox, and two radio
  rows, `By clicking anywhere on the row` / `By clicking on icon or name`
  (`@option:radio how files/folders are opened`), i.e. how a click opens an item.

The rebuild keeps its own single click model and does not expose the width steps;
both are recorded here rather than copied.

### Where the earlier "estimated" numbers stand

* name comparison default — **now measured**: `SortingChoice` default index 0 =
  `NaturalSorting` (see above); the note that called *Natural* an estimate is
  superseded;
* "show folders first" and the sort key/order defaults are **now measured** in the
  round below (`ViewProperties` ctor @0x29fdd0, `defaultSortOrderForRole` @0x273910);
  what is still estimated is only which key a brand-new folder opens with, because
  the role is not a constant: `KFileItemModel::loadSortingSettings` @0x194e20 leaves
  the model on the name comparator it builds, and a folder's own `sortRole` overrides
  it whenever the person has changed it;
* details row height 26 px, places 208 px, information panel 288 px stay
  approximations of the theme: the splitter sizes are **not** constants in the
  program — `QSplitter::setSizes` is fed either from the saved config or from the
  live widget widths (call sites at 0x90e2a, 0x927f3, 0x94673, 0x986eb), so the
  original remembers whatever the person dragged rather than opening at a fixed
  width.

## The Sort By menu, read out of the code (asked for by the person)

The person's words: *"if I choose to sort by date descending … this should carry
forward to the next time I open the app."* The sorting this build already had was
a guess in two places — clicking the key that was already chosen flipped the
order, and every key started ascending, so picking *Size* showed the smallest file
first and *Modified* the oldest date first. Neither is what the original does.

The original's sorting lives in `/usr/lib/libdolphinprivate.so.6` (the main program
`/usr/bin/dolphin` is a stripped shell around it: its own string pool holds only
`SortingChoice`, at 0x22cddc, and `SortOrderForUrl`, at 0x23b543). Strings were
found first, then their references were followed:

| string | analysed address | read from |
| --- | --- | --- |
| `SortingChoice` | `/usr/bin/dolphin` 0x22cddc | `FUN_001e7af0` (the View → Display page's write-back) |
| `Sort By` | lib 0x2bfcc6 | `DolphinViewActionHandler::createActions` @0x28e380 |
| `Folders First` | lib 0x2bf50d (`@action:inmenu Sort`) | same function |
| `Show folders first` | lib 0x2bf50d | same string, used as the settings-row wording |
| `Sort by Field` | lib 0x2bfa58 | `createFileItemRolesActionGroup` @0x28d3f0 |
| `Descending` / `ascending`, `descending` | lib 0x2bf502 / 0x2c0448, 0x2c0432 | `createActions` @0x28e380, `slotSortOrderChanged` @0x292640 |
| `Hidden Files Last` | lib 0x2c02fa (`hidden_last`) | `createActions` @0x28e380 |

Functions read with the decompiler (pseudo-code kept under `rea/deep3/`, whole-run
evidence `ev_b684ee56…fd5355` for the role default, `ev_8d30cf28…6e1aedb` for the
role list, `ev_bc293f78…8681c45` for the menu handler):

| function (libdolphinprivate.so.6) | address | what it is |
| --- | --- | --- |
| `DolphinViewActionHandler::createActions` | 0x28e380 | builds every action, including the Sort By menu |
| `DolphinViewActionHandler::createFileItemRolesActionGroup` | 0x28d3f0 | one tickable action per sortable role, role kept in the action's data |
| `DolphinViewActionHandler::slotSortTriggered` | 0x291f30 | a key was chosen: tick it, hand the role to the view |
| `DolphinViewActionHandler::slotSortOrderChanged` | 0x292640 | Ascending / Descending chosen: remember it for the current role |
| `DolphinView::setSortRole` | 0x2716d0 | applies a role and asks for that role's order |
| `DolphinView::preferredSortOrder` | 0x2859a0 | the order last chosen for a role, else its default |
| `DolphinView::defaultSortOrderForRole` | 0x273910 | is this role in the "descending first" set |
| `DolphinView::updateSortFoldersFirst` | 0x27d7c0 | writes the folders-first switch into the folder's properties |
| `KFileItemModel::loadSortingSettings` | 0x194e20 | turns the three name comparisons into two comparators |
| `ViewProperties::ViewProperties(QUrl const&)` | 0x29fdd0 | a folder's fresh defaults: order, folders-first, hidden-last |

**The menu is two radio groups and a switch, not a toggling list.**
`createActions` builds a `KActionMenu` named `sort`, titled *Sort By*
(`@action:inmenu View`, icon `view-sort`), fills it with the role actions from
`createFileItemRolesActionGroup("sort_by_")` — an **exclusive** `QActionGroup`, so
exactly one key is ticked — then `addSeparator()`, then a second exclusive group
holding *Ascending* (`ascending`) and *Descending* (`descending`), then the
*Folders First* toggle (`folders_first`, `toggleSortFoldersFirst`) and *Hidden Files
Last* (`hidden_last`, `toggleSortHiddenLast`).

`slotSortTriggered` is three lines of real work: tick the clicked action, read the
role out of `QAction::data()`, call `DolphinView::setSortRole(role)`. There is no
branch that reverses the order, so **clicking the key that is already ticked does
nothing**. Direction is only ever the pair underneath.

**A new key starts in its own direction.** `setSortRole` @0x2716d0 writes the role
into the folder's properties and then compares `preferredSortOrder(role)` with the
order in force; if they differ it stores the preferred one and pushes it into the
model. `preferredSortOrder` @0x2859a0 looks the role up in a hash of orders chosen
during the session (at `DolphinView+0x130`) and, when the role is not in it, returns
`defaultSortOrderForRole(role)`. That function is a static set built once from the
sixteen pointers at **0x2f3960** (up to `PTR_FUN_002f39e0`) and answers "is this role
in it". The sixteen strings, read out of the library:

    modificationtime  creationtime  accesstime  deletiontime  imageDateTime
    releaseYear       size          width       height        pageCount
    wordCount         lineCount     rating      duration      bitrate
    (the sixteenth pointer lands inside the string "Color Space" and reads "ace",
     so it can never match a role)

A member of that set is held **descending**; everything outside it — `name`, `type`,
`permissions`, `owner`, `group`, `path`, `tags`, `comment` — is **ascending**. So:

| key in this build | role | first direction in the original | in the build now |
| --- | --- | --- | --- |
| Name | `name` | ascending (A → Z) | ascending |
| Size | `size` | **descending** (biggest first) | descending |
| Type | `type` | ascending | ascending |
| Modified | `modificationtime` | **descending** (newest first) | descending |

**A direction chosen by hand stays with that key.** `slotSortOrderChanged`
@0x292640 ticks the two actions and calls `DolphinView::setPreferredSortOrder(role,
order)`, i.e. it writes the order into that per-role hash — for the current role
only. Switching to another key and back therefore comes back to the direction the
person last gave it, and a key that was never touched falls back to the table above.

**Folders First and the two fresh-folder defaults.** `updateSortFoldersFirst`
@0x27d7c0 stores the switch in the *current folder's* view properties
(`ViewProperties::setSortFoldersFirst`) and calls
`KFileItemModel::setSortDirectoriesFirst`. A folder that has never been given one
takes the `ViewProperties` constructor's defaults, and the constructor @0x29fdd0
writes the three bytes `0, 1, 0` at `this+8, +9, +10`: **sort order ascending, folders
first on, hidden files last off**. That is where "folders first is on by default"
comes from — the earlier sheet only guessed it.

**What the three name comparisons really do.** `loadSortingSettings` @0x194e20
reads `GeneralSettings+0xac` (`SortingChoice`) and sets two things: a natural-sort
flag at `this+0x60` and the `QCollator`'s case sensitivity at `this+0x58`.

| SortingChoice | natural compare | collator case |
| --- | --- | --- |
| 0 `NaturalSorting` (default) | **on** | insensitive |
| 1 `CaseInsensitiveSorting` | off | insensitive |
| 2 `CaseSensitiveSorting` | off | sensitive |

So *Natural* is not "the byte order with digits handled": it is natural order **with
case folded**, which is what `state::NameSort::Natural` does through
`fsutil::name_cmp`. The two alphabetical modes differ from each other only in that
one collator flag; this build compares case-folded strings and exact strings
respectively, because a `QCollator` is ICU-backed and the rebuild does not ship ICU —
that difference is recorded as an estimate, and only for accented letters.

**What changed in the build** (`app/src/state.rs`, `app/src/main.rs`):

* `SortKey::descends_first()` carries the table above, so `Pane::choose_sort` opens
  Size and Modified biggest/newest first and Name and Type A-first;
* `Pane::preferred_desc` is the per-key memory (`[Option<bool>; 4]`), written by
  `Pane::set_sort_dir` and read by `choose_sort` before the default — the same order
  of preference as `preferredSortOrder` → `defaultSortOrderForRole`;
* choosing the key that is already chosen no longer flips anything (`choose_sort`
  returns early), and the popup now has the original's *Ascending* / *Descending*
  pair under the key list, then the *Folders First* switch with the original's own
  wording (it used to read "Show folders first");
* `--selftest` gained three checks: the per-key first direction, "a second click on
  the chosen key keeps the order", and "a direction chosen for one key is still
  there when you come back to it".

Pictures of the new popup: `app/shots/sort/menu.png` and `sort/descending.png`.





## The name on the window (asked for by the person)

The person saw the window title still reading "Dolphin" and asked for
"DolphinCraft". The original does not paint its name inside the window; the name
lives in the title bar the window manager draws, and comes from the program's own
declared application title.

| what | original | this build | where it comes from |
| --- | --- | --- | --- |
| title bar text | `Dolphin` | `DolphinCraft` | original: the NUL-terminated string `"Dolphin"` at **0x12d350** in `/usr/bin/dolphin`, immediately after the `@title` tag of its KAboutData block (next to `"File Manager"` and the authors) — i.e. the program's declared name, not a translated label. Rename asked for by the person. |
| `WM_CLASS` / app id | `dolphin` | `dolphincraft` | the original's desktop file / QGuiApplication application id; the copy follows the new title so the task switcher groups it as its own app |
| name shown in the About row | — | `DolphinCraft — file manager with extra tools` | rebuild's toast; the original's About box is a KAboutData dialog, which this build does not reproduce |

Both the viewport title and the name handed to `eframe::run_native` were changed
together, so the X11 `_NET_WM_NAME` and the Wayland title agree. Checked on a
virtual X display: `xprop` on the running window reports
`_NET_WM_NAME(UTF8_STRING) = "DolphinCraft"` and `WM_CLASS = "", "dolphincraft"`,
and `xdotool search --name DolphinCraft` finds exactly that window. The saved
settings folder is deliberately left at its old name (`dolphin-remake`) so the
person's shortcut colours, order and window size survive the rename.

Nothing else in the window changed: the client area never carried the name, which
is why the screenshot check cannot see the rename — it is a title-bar change.
Estimated, in the sense that it cannot be read from the original's code: how the
title bar ellipsises a long name on a narrow window is the window manager's
business, not the program's.

The name lives in one place in the code (`APP_NAME` in `app/src/main.rs`); the
viewport title, the name handed to the window library, the app id and the About
row all read it, and `--selftest` has a case that fails if it stops being
`"DolphinCraft"`. The application-menu shortcut that was installed for this build
was renamed to match, and its window-class hint now says `dolphincraft` as well,
so the task bar groups the window under the right name instead of the original's.

## The picture wall, the viewer, the slideshow and the comparison (asked for by the person)

This is the second batch of additions: a wall of picture cards inside the file
manager, a full-window viewer that reads a generated picture's prompt out of the
file, a full-screen slideshow with a switchable fade, and a side-by-side picture
comparison.

None of these exist in the original. Checked, not assumed: the string table of
`/usr/bin/dolphin` and of `libdolphinprivate.so.6` has no "slideshow", no
"full screen" viewer and no image comparison — its only "Compare" is
`compareFiles` / "Compare Files", which diffs two text files. So there is no
original code to decompile for these features, and no dolphin constant to copy.
The numbers come from the person's own image browser
(`~/personal-files/python-projects/image-browser-styled`), which is what they
were comparing against; each value below was read from those sources, not
guessed, and the sources are the ones the browser uses at runtime.

| what | value | where it comes from |
| --- | --- | --- |
| thumbnail size slider | 50 … 400 px, first value 200 px | `templates/browser.html` `#sizeSlider` `min="50" max="400" value="200"` |
| grid gap between cards | 12 px | `static/css/browser.css` `.images { gap: 12px }` |
| padding inside a card | 8 px | `static/css/browser.css` `.image { padding: 8px }` |
| picture corner radius ("rounded") | 10 px | `static/css/browser.css` `.image img { border-radius: 10px }` |
| dimension line | `W×H:AR a.bc:MP d.dd` | `templates/browser.html` line 203, the `show_dims` block: `{{ width }}×{{ height }}:AR {{ aspect_ratio }}:MP {{ megapixels }}` |
| slideshow interval | 1.5 s first (person's request; the browser starts at 1 s), 0.5 s steps | the person asked for a 1.5 s default on 2026-11-07; the browser's `#slide-interval` is `min="0.5" value="1" step="0.5"` |
| slideshow fade | 300 ms | `static/js/browser.js` `startSlideshow`, the second `setTimeout(..., 300)` around the picture swap |
| comparison modes | side, split, overlay | `#compare-modal` toolbar |
| comparison overlay opacity | 50 % first | `#compareOpacitySlider` `min="0" max="100" value="50"` |
| comparison zoom | ×1.25 / ÷1.25 buttons, ×1.1 / ×0.9 per wheel notch, clamped 0.1 … 20 | `compareZoomIn`, `compareZoomOut`, `initCompareZoomWheel`, `stepCompareZoom` |

The original's own picture size ladder was measured for contrast: it stops at
**256 px** (`ZoomLevelInfo::maximumLevel()` @0x1a6530 returns 16 and
`ZoomLevelInfo::iconSizeForZoomLevel(int)` @0x1a6540 returns `level * 16` above
level 4), so a 400 px ceiling is an addition, not a copy. The 30 s upper limit of
the slideshow, the fade switch, and the "Hide names / Show dimensions / Rounded"
toggles as separate switches are this build's own: the browser carries those
choices as URL parameters (`hide=1`, `dims=1`) and always fades, so there was no
number to read for them. They are recorded as such in `mechanics.json`.

Two things the browser does that this build deliberately does not: its
"explode" folder mode (folders opened into the grid) and its browser-page
navigation buttons. Folders simply sit in the wall as folder cards, and the
file manager's own path bar does the navigation.

The prompt reader is not from the browser either — it reads the file format
itself: the PNG `tEXt` chunk named `parameters` (A1111 / Forge) or
`workflow`/`prompt` JSON (ComfyUI), and the JPEG `Exif` `UserComment`
(A1111-written JPEGs). Fields lifted out: prompt, negative prompt, steps,
sampler, CFG scale, seed, size, model, denoising strength. A picture with none
of these shows no Prompt panel rather than invented text.

Verified in the running window (pictures in `app/shots/final/`): `wall400.png`
(400 px cards), `wall-dims.png` (the dimension line under each name),
`viewer.png` (the Prompt panel with the 9 fields read from a hand-built PNG),
`compare3.png` (two pictures side by side with the Side/Split/Overlay bar).

## This pass: the 400 px ceiling and the picture switches in the file view

The person reported that thumbnails stopped at 256 px and that the rounded
corners and the other switches were missing. Both were real gaps in the last
build, and both are now closed where the person browses pictures.

**Where the 256 px came from.** It is the original's own ceiling, measured
again from the installed library:

| function | address | result |
| --- | --- | --- |
| `ZoomLevelInfo::minimumLevel()` | 0x1a6520 | 0 |
| `ZoomLevelInfo::maximumLevel()` | 0x1a6530 | **16** |
| `ZoomLevelInfo::iconSizeForZoomLevel(int)` | 0x1a6540 | levels 0..4 from the table at 0x1cd980 (16, 22, 32, 48, 64), level 5 and up `level * 16` |
| `DolphinItemListView::setZoomLevel(int)` | 0x26e100 | clamps the incoming level to the pair of bounds at 0x1c3350, i.e. 0..=16 |

Level 16 is therefore 256 px, and that is the largest preview the original can
draw. The previous build copied that clamp into the file views (`clamp(16, 256)`
in the icon, compact and details drawing code) *and* stopped the zoom ladder at
level 16, so the added 400 px ceiling only existed inside the picture wall.

**What changed.** The ladder in `theme.rs` now continues the original's own
formula instead of stopping early: levels 17..=25 are 272, 288, 304, 320, 336,
352, 368, 384, 400 px. Nothing at or below level 16 moves, so every measured
size the original offers is unchanged; the four `clamp(16, 256)` calls became
`clamp(16, 400)`, and `Pane::zoom_by` clamps to the new top. `zoomLevelForIconSize`
(the inverse, 0x1a6560) still maps 400 px back to level 25, so a saved size
round-trips. `--selftest` covers both directions.

**The switches.** `Names`, `Dimensions`, `Rounded` and `Previews` were only
reachable inside the picture wall. They are now also drawn in a one-row strip
above the icon grid whenever the folder holds at least one picture, and they
are repeated under the app menu's *Pictures* submenu, so they can be found
without knowing the wall exists. The flags themselves are the same pane fields
the wall uses (`gallery_names`, `gallery_dims`, `gallery_round`, `previews`),
which are saved in the settings file, so a choice holds across folders and
restarts.

* Corner radius in the file views is `clamp(icon * 0.035, 3, 10)` px: the wall's
  own 10 px (`PICTURE_CORNER`, from the browser's `.image img { border-radius:
  10px }`) at the 400 px top, scaled down so a 32 px icon is not rounded into a
  circle. The mask is painted in the colour actually behind the picture, so a
  selected or hovered cell keeps its tint in the corners.
* The dimension line is the wall's own caption format
  (`W×H:AR a.bc:MP d.dd`), drawn under the name in the name's own font, size
  and colour (see "The size line is written like the name" below), and the
  icon cell grows by a measured caption line when it is on so rows never
  overlap.
* Names off hides the label but keeps the cell, so pictures do not jump around
  when the switch is flipped.

Values are repeated as data in `mechanics.json` (`icons view` and `picture wall`
groups) with the addresses above as their source; the extended ladder's top is
marked as an addition, not as something the original does.

## The size line is written like the name (asked for by the person)

The person's words: *"the dimensions displayed should have the same colour, size
and font as the filenames"*.

Until this pass the two lines under a picture were drawn as two different
things. The name was 12 px in the ordinary text colour; the size line was
10.5 px in the faint colour, one size smaller and a shade dimmer, and in the
picture wall it also sat on a shorter line step (15 px against the name's 17).
The words are the same kind of caption, so they now share one definition:

| what | value | where |
| --- | --- | --- |
| caption font | proportional | `gallery::caption_font()` |
| caption size | **12 px** (the file name's own size) | `gallery::CAPTION_SIZE` |
| caption colour | `TEXT` (`#EFF0F1`), lifted to white on a marked cell | `gallery::caption_colour()` |
| caption line step | **17 px**, name and size alike | `gallery::CAPTION_STEP` |
| icon-grid size-line offset | 14 px below the last name line | `views::draw_icon_cell` |

* The names already used this font and size, so the *name* is unchanged; only
  the size line grew and brightened. The 10.5 px faint style is gone from both
  places a picture's size is printed under it — the icon grid (`views`) and the
  picture wall (`gallery::draw_gallery_card`).
* A marked cell still turns its whole caption white, name and size together, so
  one line never reads differently from the other.
* Room: the wall's caption block is now 17 px per line (`caption_height`), and
  the icon grid measures a caption line from the font itself and grows the cell
  by that much (`dims_extra`) instead of the old fixed 15 px that was sized for
  the smaller text. Nothing is clipped at 192 px or at 400 px.
* The information panel's `Dimensions:` row is a panel row, not a caption under
  a picture, and keeps the panel's own label/value styling.

`--selftest` checks the shared font, size, colour and line step, and
`mechanics.json` repeats them under the `picture wall` group with the person's
request as their source.

## The slideshow starts at 1.5 s (asked for by the person)

The person asked that the slideshow's default interval be 1.5 seconds. The
slideshow is an addition — neither `/usr/bin/dolphin` nor
`libdolphinprivate.so.6` contains the word or a timer that advances pictures, as
the string search recorded in the section above shows — so this number comes
from the person's request, not from the original's code. The image browser the
feature was modelled on starts its own `#slide-interval` at 1 s with 0.5 s steps;
only the starting value changes here.

* `SLIDE_DEFAULT` is 1.5 s (`app/src/gallery.rs`). It is the value a fresh
  settings file gets, and the value an older settings file gets when it has no
  interval key, because `config.rs` seeds that field from the same constant.
* A settings file written before this change still holds the old 1 s default.
  `Config::migrate` lifts exactly 1.0 s to 1.5 s once (`settings_version` goes
  from 0 to 1) and leaves any other saved interval alone, so a person who chose
  a different timing keeps it and a person who never touched the slider sees
  the new default.
* The slider runs 0.5 … 10 s in 0.5 s steps, so 1.5 s is one of its stops and the
  label above it reads `1.5s`; the clamp still accepts 0.5 … 30 s for a value
  restored from settings.
* The fade is unchanged at 300 ms and still starts on.
* Starting a show with Space or the player button now uses the interval and fade
  the person last set (`Slide::remembered`) instead of silently resetting them to
  the factory value; the 1.5 s default is therefore a first-run value, not a
  reset on every play.
* `--selftest` checks that the fresh interval is exactly 1.5 s, that a picture
  has not moved at 1.4 s but has at 1.6 s, and that a restarted show keeps a
  chosen 4 s interval with the fade off.

`mechanics.json` repeats the value under the `slideshow` group with the
person's request as its source.


## The prompt text block: selecting past the edge (asked for by the person)

The person's words: *"the prompt selection options should allow text to be
selected taking scrolling into account"*. Until this pass the prompt was a
read-only `TextEdit` inside a `ScrollArea`: the field clipped its own text, so
a drag could only ever reach the rows that happened to be on screen, and the
wheel scrolled the whole information panel out from under the pointer. Nothing
in `/usr/bin/dolphin` answers this, because the original has no prompt reader at
all — the whole block is an addition built for this remake — so the rules below
are this build's own, written down here so the checks and the sheet agree.

What the block does now (`app/src/selectable.rs`):

* The **whole** text is laid out once at the panel's width (proportional 11.5 px,
  wrap width = available − 2 × 6 px padding − 12 px scroll gutter) and the block
  owns its scroll offset. Selecting therefore works on every character, on
  screen or not.
* Dragging to within **12 px** of the top or bottom edge scrolls the block
  **10 px per frame** (≈600 px/s at 60 Hz) in that direction and keeps extending
  the selection, so a prompt longer than its box can be selected from the first
  word to the last in one gesture. The same drag can reverse direction and take
  the selection back.
* The wheel scrolls the text and **keeps the selection**: a click at the top, a
  wheel scroll and a Shift+click at the bottom join into one range. Shift+click
  moves the far end instead of restarting; a plain click restarts.
* Double-click selects the word under the pointer (letters, digits, `_`, `-`,
  `'`, `/` count as one word); Ctrl+A selects everything, Ctrl+C copies the
  selection, Esc drops it, and the right-click menu offers *Select all* and
  *Copy selection* so the keys do not have to be remembered.
* The box is capped at **150 px** for the prompt and **96 px** for the negative
  prompt, and its height is snapped to whole text rows, with the paint clipped
  one padding step above the bottom edge, so no row is ever left half-drawn.
* A hairline thumb down the right edge shows which part of the text is in view;
  it is drawn by the block itself because the box hides the scroll area's own
  bar to keep the text width predictable.
* The keys only act while the block has the focus, so Ctrl+A still selects the
  files in the list when the prompt is not focused.
* The row of options above the text (*Select all*, *Copy prompt*, *Copy all*,
  *Save*) wraps onto a second line when the column is too narrow to hold all
  four, so no option is ever clipped in a small window or a narrow Information
  panel.

`--selftest` drives the real widget headlessly for both halves of this: a drag
held on the bottom edge scrolls the box and copies from the first character to
the last (1 492 of 1 492 characters), and a click + wheel + Shift+click copies
from the first click to the last row after 174 px of scrolling.

`mechanics.json` repeats the numbers under the `prompt` group.

## The picture wall keeps one size for every row (asked for by the person)

The person's words: *"I noticed with the 'pictures - thumbnails of this folder'
view set to 400 px, the last row seems to be displayed in 400px but the rest is
not."* Measured on this build's own window picture before the change
(`app/shots/wall400-now.png`, five pictures, the slider at 400 px, a 1440 × 900
window whose wall is 1202 px wide):

| row | pictures in it | picture height drawn |
| --- | --- | --- |
| 1 | landscape 16:9, portrait 3:4, portrait 9:16 | **381 px** |
| 2 | scan-tall 1:2, square 1:1 (the leftover row) | **400 px** |

The cause was the packer, not the slider. `wall_rows` used to justify every
row: the height that makes the row exactly fill the window width was computed
from the sum of the aspects, and the row took whichever of "one more picture" or
"one fewer" came closer to the ask. A row of three pictures therefore landed
where the width said, not where the slider said — 1202 / (1.78 + 0.75 + 0.5625)
= 381 px — while the last row, which nothing had to justify, kept the full 400.
The same rule could also blow a short last row up to 1.6 × the slider height
(`ROW_STRETCH_MAX`), so the one row everybody notices at the end of a folder was
the one row that never matched the number on the slider.

**What the wall does now** (`app/src/gallery.rs`, `wall_rows`):

* every row is packed at **exactly** the slider's height, the last row included;
  a row takes pictures while `aspect × height + gap` still fits the wall width,
  and ends at the first picture that does not. The wall is a size, not an
  average;
* a picture is still as wide as its own shape, so the 12 px gap is still the only
  space between two pictures and the frames still follow a wide or a tall shot;
* the right-hand edge of a row may now stop short of the window edge — the price
  of one honest size for every row, and what any fixed-size thumbnail grid does;
* the one exception is a picture wider than the whole wall at that size (a
  stitched panorama, up to 6:1 = 2400 px at 400 px): alone in its row it is drawn
  as wide as the wall, so it comes out shorter than the rest. `ASPECT_MIN` /
  `ASPECT_MAX` (1:5 … 6:1) still hold the shapes;
* the 50–400 px range, 200 px at first, the 12 px gap, the round corners and the
  captions are unchanged (see the values in `mechanics.json`).

`--selftest` drives both halves: *"every row of the picture wall is the size the
person set, the last one too"* packs twelve mixed pictures into a 1400 px wall at
400 px and refuses to pass unless every row measures 400 px and no row is wider
than the wall; *"every picture is as wide as its own shape"* keeps the old
guard that no row overflows.

Measured after the change (`app/shots/wall400-fixed.png`, same folder and
window): row 1 = 398–400 px tall, row 2 = 400 px tall.

## The picture wall holds pictures only (asked for by the person)

The person's words: *"in the \"Pictures - thumbnails of this folder\" view only
pictures should be displayed."* That view is this build's own picture wall
(`app/src/gallery.rs`, `ui_pictures`), reached from the picture button in the
toolbar or **F4**; the original Dolphin has three view modes only — Icons, Compact
and Details. `DolphinView::setViewMode` (`@0x2710c0`) does nothing but store the
mode at `this+0x58` and hand it to `ViewProperties`; the mode names in
`ViewModeController` are exactly those three, and the only mention of browsing
pictures is the icons view note, "handy to browse through pictures when the
Preview option is enabled", in the documentation string table of
`/usr/lib/libdolphinprivate.so.6`. There is therefore no original listing rule to
copy here, and the wall's contents are decided by the person's request.

**What the wall does now** (`app/src/state.rs`, `Pane::shown_indices`;
`app/src/gallery.rs`, `ui_pictures`):

* the wall draws `shown_indices()` filtered to `!is_dir && is_image()`: a
  sub-folder or a document is not drawn at all. Before, the filter was
  `e.is_dir || e.is_image()`, so a folder sat on the wall as a square tile and a
  wall of letter tiles was possible in a folder of documents;
* a picture is recognised by its file name's extension, the same list the
  previews, the viewer and the slideshow already use: `png jpg jpeg gif bmp webp
  tif tiff svg ico heic avif xpm ppm` (`app/src/fsutil.rs`, `kind_for`). A symlink
  whose name ends in one of those is a picture too; a symlink to a folder is a
  folder and stays off the wall;
* hidden pictures still obey the *Show hidden files* switch, and the find bar
  still narrows the wall to the pictures whose names match, because the wall
  starts from `visible()` and then keeps the pictures;
* a folder with no pictures shows *"No pictures in this folder"* and *"The wall
  shows pictures only; the list views show every file."* The path bar above the
  wall is how the person steps into a sub-folder now that folders have no tile.

**The selection follows the wall.** Filtering the drawing was not enough: Ctrl+A,
Shift+click and the arrow keys walked the whole folder listing by index, so a
document hidden behind the wall could still be marked — and Delete would then
remove a file the person could not see. `Pane::shown_indices()` is now the one
list every one of those paths uses (`select_all`, `select_range`, `App::
move_cursor`), and `Pane::set_pictures(true)` drops any mark the wall does not
show, which is where the toolbar button, the app-menu row, the settings panel's
*Pictures (Gallery)* switch and the **F4** request all arrive.

`--selftest` checks it with a folder holding two pictures, a text file, a video
file and a sub-folder: *"the picture wall shows pictures and nothing else"*
requires exactly the two pictures on the wall, Ctrl+A to mark exactly two items, a
Shift+click from a document to mark the one picture clicked, and a folder marked
before the wall came on to be unmarked after it. Window pictures:
`app/shots/wall-only.png` (13 pictures, the 12-photo roll plus the prompt sample,
with `notes.txt`, `budget.csv` and `Subfolder` in the same folder but not on the
wall) and `app/shots/wall-none.png` (a folder with one document: the empty-wall
message).

## The prompt area folds away (asked for by the person)

The person's words: *"the prompt area should be collapsable as it is not always
needed."* The block that shows the recipe a picture was made with (fields,
prompt, negative prompt) used to take the whole right-hand column of the viewer
and a fair slice of the Information panel, whether or not the person was
interested in it that day.

**What it does now** (`app/src/gallery.rs`, `prompt_panel` / `prompt_header`):

* the block's title row is one hit target, **22 px** tall: a fold arrow, the word
  *Prompt* and, on the right, the tool that wrote the recipe (`parameters`,
  `Stable Diffusion` …). The arrow points down while the block is open and right
  while it is folded;
* a click folds it to the title row plus **one elided line** of the prompt, so a
  folded block still says what it holds; the fields, the *Copy prompt* / *Copy
  all* / *Save* buttons, the selectable text box and the negative prompt go away
  with it;
* the same switch sits in the picture viewer on the **P** key, next to **I** for
  the whole details panel, and both write the same setting; the status line says
  *Prompt folded away* / *Prompt shown*;
* the choice is kept between runs (`prompt_open` in the settings file, on by
  default), so a person who never wants the recipe sees a one-line block from
  then on;
* `--selftest` clicks the real title row headlessly and checks the text box
  disappears on the first click and comes back on the second.

`mechanics.json` repeats these numbers under the `prompt` group.

## The picture details panel starts closed (asked for by the person)

The person's words: *"the picture details panel should be closed by default."*
Until this change the viewer opened the details panel by itself the first time a
picture carrying a generation recipe came up, so the recipe covered the
right-hand third of the picture whether or not it was wanted.

**What it does now** (`app/src/gallery.rs`, `Viewer::sync_meta`):

* the panel is shut every time a picture viewer opens, including from the
  command line and for slideshows and for a picture that carries a prompt;
* the recipe is still read behind the panel — `sync_meta` fills `Viewer::meta`
  once per picture, so opening the panel shows the prompt with no wait;
* the three ways in are the info button at the right of the viewer's top bar,
  the **I** key, and **P**. **P** now opens the panel first and folds the prompt
  inside it on the next press, because with the panel shut by default the old
  rule (fold only when the panel is open) made P do nothing on a fresh picture;
* paging from one picture to the next leaves the panel the way the person left
  it — shut if shut, open if open;
* the main window's Information panel was already off unless switched on (its
  `info_panel` default is false and nothing opens it on its own), so this change
  is the viewer's own panel only.

`--selftest` opens the viewer on a picture that carries a prompt, lets it read
the recipe exactly as the window does, pages to a second picture, and refuses to
pass if the panel comes up open at either point: *"the picture details panel is
closed when a picture opens, and stays closed when paging"*.

Measured after the change (`app/shots/details-closed2.png`, a 1200 × 760 window
on `testdata/mixed/prompt-sample.png`): picture fills the canvas, no panel, the
status line reads *832 × 1216 · 0.68:1 · 1.01 MP · 117.1 KiB*.

Two faults found while photographing it, both fixed in the same pass:

* **the size line could read nonsense.** The pixel size was read from a fixed
  offset, assuming the PNG image header was the first chunk. A picture whose text
  chunk came first gave *1885434465 × 1835365477 · 1:03:1 · 3460461326206.96 MP*
  (`app/shots/details-closed.png`, before). `search.rs` `png_size` now walks the
  chunk list to the `IHDR`, up to 64 chunks, stopping at the pixel data;
* **a picture the decoder refuses looked like one that was still arriving.** The
  viewer said *Loading picture…* for ever. `BigCache::has_failed` now lets it say
  *This file could not be opened as a picture*
  (`app/shots/details-closed4.png`).

`mechanics.json` repeats these under the `picture viewer` group.

## The side-by-side comparison: zoom and pan (reported by the person)

The person's words: *"in the side by side comparison view, the zoom and pan
behaviour is not working properly."* Four separate faults were behind that, and
all four were in this build's own compare view, not in the original program. The
comparison is an addition modelled on the person's image browser
(`~/personal-files/python-projects/image-browser-styled`), so its rules are that
browser's, read from its own functions:

| rule | value | where it comes from |
| --- | --- | --- |
| wheel zoom | ×1.1 in, ×0.9 out | `static/js/browser.js` `initCompareZoomWheel` → `stepCompareZoom(1.1 / 0.9, …)` |
| button zoom | ×1.25 / ÷1.25 | `compareZoomIn` / `compareZoomOut` |
| zoom range | 0.1 … 20 | `stepCompareZoom` clamp |
| which picture the wheel acts on | the pane under the pointer | `initCompareZoomWheel`, `e.target.closest('.compare-pane-a, .compare-pane-b')` |
| which picture a drag acts on | the pane the drag started in | `initComparePan`, same closest() test |
| Sync | side mode follows the switch; split and overlay are **always** synced | `isCompareSynced` returns true for `split-mode` and `overlay-mode` whatever the checkbox says |
| synced drag | the drag writes picture A and the copy is pushed to B | `initComparePan`, `if (isCompareSynced()) { … _compareZoomB.offsetX = _compareZoomA.offsetX … }` |
| read-out | one value when synced, `A% / B%` when not | `updateCompareZoomLabel` |

**Fault 1 — one wheel notch ran the zoom to its ceiling.**
`app/shots/cmp/before/01-wheel-right.png`: two notches took the read-out from
100 % to **2000 %**, the top of the 0.1 … 20 range. The handler read
`i.smooth_scroll_delta.y + i.raw_scroll_delta.y`; egui spreads one notch over a
dozen frames, and every frame that still had a delta bought a full step. Fixed
by reading the wheel as **events** (`gallery::wheel_units_in_frame`) and turning
them into steps through the same 120-units-per-notch arithmetic the file view
already used (`state::wheel_units`, `state::wheel_zoom_steps`). Measured in the
running window: one notch now gives 100 % → 110 %, two give 121 %.

**Fault 2 — one notch was counted twice.**
Even with events, two notches gave 146 % (four steps). The debug log shows one
physical notch arriving as **two line events 7 ms apart** — the press and the
release of X11 wheel button 4 — so `gallery::Wheel::steps` counts a notch once and
ignores a second line event of the same sign inside **20 ms**
(`WHEEL_DUPLICATE_S`). A touchpad's pixel-based events are still summed, so a
40 px glide is one step, exactly as in the file view.

**Fault 3 — the pictures were not clipped to their panes, so one hid the other.**
`draw_fit` painted the picture's rectangle with no clip rect: at 2000 % the
right-hand picture covered the whole window, and `app/shots/cmp/before/02-drag-right.png`
is a flat blue wash with the names still printed underneath. Every pane is now
painted through `Painter::with_clip_rect`.

**Fault 4 — the zoom drifted, the drag ignored the pointer, and a picture could
be lost.** In Side mode both pictures were drawn from one wheel/one drag over the
whole canvas; the second picture's pan was never written at all, so with Sync off
it could be zoomed but not moved, and with Sync on it stayed still while the first
one moved. The zoom was anchored on the middle of the **window**, so a notch over
the right-hand picture pushed the picture sideways. And nothing stopped a drag,
so the picture could be thrown out of the window entirely.

The rules the rebuild now keeps (`app/src/gallery.rs`):

* **one drag area per pane** (`cmp_pane_a`, `cmp_pane_b`, and for Split the two
  halves of the frame plus the divider's own strip), so a drag moves the picture
  it was started on;
* **the zoom is anchored on the pointer** (`gallery::zoom_about(zoom, pan,
  anchor, factor)`): a point `q` of the picture sits at `pan + q·zoom` from the
  pane's middle, so the new pan is `anchor − (anchor − pan) · (z′ / z)`. The point
  under the pointer does not move;
* **the pan stops at the picture's edges** (`gallery::clamp_pan`, limit
  `(|size| − |pane|) / 2`, never negative): a picture that still fits cannot be
  dragged, and one that is bigger can be pushed exactly until an edge meets the
  pane edge;
* **Sync means Sync**: `gallery::compare_synced(mode, sync)` is false only for
  Side with the switch off; Split and Overlay always move both pictures, as
  `isCompareSynced` does. While synced, the partner's zoom and pan are kept equal
  to the first picture's, so turning Sync off starts from the same framing;
* **the read-out names both pictures** while they are separate: `100% / 121%`;
* **the divider in Split is its own handle** (a 20 px strip with a grip dot), and
  the two drag areas stop 10 px short of it, so grabbing the divider never pans a
  picture and panning never moves the divider. The two names sit on dark tabs
  beside it;
* **a new comparison starts framed**: opening Compare, swapping the pair and
  choosing another mode all go back to 100 % in the middle, which is what
  `swapCompareImages` and `setCompareMode` do (`resetCompareZoomInternal(true)`);
* **paging keeps the pair together**: ← and → now move both pictures and their
  names, instead of leaving the old pair behind (the names under the pictures had
  been stale);
* the zoom buttons, the `+` / `−` keys and `0` (Fit) act on the first picture and
  carry the partner with them while synced, as the browser's buttons do.

**Checked by driving the real window** (virtual display, virtual mouse; pictures
in `app/shots/cmp/`):

* `cmp/after5/01-wheel-right.png` — two notches = **121 %**, both pictures still in
  their own halves;
* `cmp/side/01-right-zoomed.png` — Sync off, wheel over the right picture: read-out
  `100% / 121%`, the right picture grew, the left one did not;
* `cmp/side/02-right-dragged.png` — a drag on the right picture moved only the
  right picture; `03-left-zoomed.png` — `121% / 121%`, each panned on its own;
* `cmp/side/04-right-clamped.png` — a 700 px drag leaves the picture filling its
  pane, edge to edge, with no gap;
* `cmp/split2/01-zoomed.png` — Split: one frame across the divider, a grip dot and
  two readable name tabs; `cmp/split/03-divider.png` — the divider follows the
  pointer while both pictures keep their shared framing;
* `cmp/overlay2/01-zoomed.png` — Overlay: both pictures at the same zoom and pan,
  the caption on a dark tab;
* `cmp/one/01-two-notches.png` — the single-picture viewer takes the same event
  path, 121 % after two notches, and `02-drag-far.png` shows its clamp.

`--selftest` covers the arithmetic without a window:

* *"a wheel notch over a picture is exactly one zoom step"* — the two halves of one
  notch give 1 then 0 steps, a later notch gives 1, and two notches come to 121 %;
* *"zooming keeps the point under the pointer in place"* — the anchored point moves
  less than 0.01 px;
* *"a drag cannot throw a picture out of its pane"* — a 500 px drag on a picture
  that fits is refused, and at 200 % the pan stops at the edge;
* *"only the side-by-side pair has a Sync switch"* — Split and Overlay ignore it.

Estimated, and marked so in `mechanics.json`: the **20 ms** duplicate-notch window
(the press/release gap measured here was 7 ms; a real fast wheel could in
principle repeat inside 20 ms and lose one step) and the **pan clamp** itself —
the browser translates freely and can lose a picture, so the stop at the picture's
edges is this build's own rule, chosen to answer the person's report.

## Open With: the menu rows, the chooser, and remembering (asked for by the person)

The person asked for the everyday Dolphin menu on a file, not just the file
operations, so this pass added "Open With" to the right-click menu. Everything
below was read out of the libraries the original loads, then checked against
the original running under a virtual display (`checks/openwith-orig/`,
`file-menu.png` and `menu.png`).

**The strings, and the code that loads them** (`libKF6KIOWidgets.so.6.31.0`, the
file behind `/usr/lib/libKF6KIOWidgets.so.6`; offsets verified with
`strings -t x` and the references with `objdump -d`):

| string | address | loaded at | enclosing function |
| --- | --- | --- | --- |
| `&Open With` (submenu row) | 0x11c24a | 0x64b5e | 0x646d0 |
| `Open &with %1` (preferred row) | 0x11c258 | 0x64d34 | 0x646d0 |
| `&Open With` (window title) | 0x11c37c | 0x6e1ca | 0x6d980 |
| `&Other Application` (last row) | 0x11c387 | 0x6e028 | 0x6d980 |
| `&Remember application association for all files of type "%1" (%2)` | 0x120210 | 0x84731 | 0x834c0 |

So the menu is two functions, not one: 0x646d0 builds the rows inside the
right-click menu, 0x6d980 is the chooser window, and 0x834c0 draws the tick box
that writes the association. The `%1` in `Open &with %1` is the program's own
`Name=`; the `%2` in the tick box's sentence is the file type.

**The rules the rebuild keeps** (`app/src/apps.rs`, `app/src/panels.rs`,
`app/src/views.rs`):

* the first row is *"Open with <program>"*, naming the desktop's own preferred
  program for the file's type, read from `[Default Applications]` in
  `$XDG_CONFIG_HOME/mimeapps.list` — the same file every other program on the
  computer reads, so a choice made here is a choice made everywhere;
* below it *"Open With ▸"* lists the programs whose `.desktop` entry names the
  type in `MimeType=`, in name order, followed by *"Other Application…"*;
* that last row opens a chooser holding **every** program the desktop has: the
  ones that claim the type first, then the rest, each half in name order, with
  the text field narrowing on the name and ↑/↓, Enter and Escape working
  without the mouse. The desktop's preferred program is the row it opens on;
* the tick box is worded exactly as the original's, with the extension and the
  type filled in: *Remember this choice for all files of type ".md"
  (text/markdown)*;
* ticking it rewrites **one** line of `mimeapps.list` — the row for that type in
  `[Default Applications]` — and leaves every other row, every other section and
  the file's order alone, which is what makes the first row of the menu change
  the next time it is opened;
* `Exec=` is turned into arguments with the desktop-entry field codes filled in
  (`%f`, `%F`, `%u`, `%U`, `%i`, `%c`, `%k`, `%%`); a program with no file code
  gets the paths appended, so nothing opens empty;
* a file whose name says nothing (`Cargo.lock`, `Makefile`, a name with no
  extension) is read: the first kilobyte of plain text is `text/plain`, and a
  NUL or a control byte keeps it `application/octet-stream`. Without that,
  right-clicking a text file with an unusual extension offered *no* program at
  all, where the original offers editors — the type a desktop reports comes from
  the file's content as well as its name.

**Checked in the running window** (`app/shots/`): `ctx-file.png` is the file
menu with *Open* / *Open With ▸*; `ctx-submenu.png` is the submenu on
`Cargo.lock` reading **Kate, KWrite, Micro, Vim, Other Application…**;
`openwith3.png` is the chooser, opened on a `.md` file, led by the one program
that claims `text/markdown` and then every other program by name.

`--selftest` covers the parts without a window: *"a program's entry says what it
opens, and a hidden one is left out"*, *"the chosen files reach the program, and
no field codes are left behind"*, *"remembering a program rewrites only the row
for that kind of file"*, *"the tick box names the file type the way the original
does"*, *"the menu finds the programs this computer has for a kind of file"*,
*"a file whose name says nothing is read before its menu is built"* and *"the
chooser leads with the programs that open this kind of file"*.

Estimated, and marked so in `mechanics.json`: the **1024-byte** sniff window (the
original's test is shared-mime-info's, whose own rule is a priority question and
not a fixed length), and the **12-row cap** on the submenu — the original's
submenu was not measured with more than a handful of programs installed, so the
cap is this build's own choice to keep the menu on screen.

## The delete question: read out of Dolphin and out of KIO (this pass)

This round the original's **main program** `/usr/bin/dolphin` (26.08.2, sha256
`341e417b…fad937`) was opened again, and because the first three delete strings
turned out to be only half the story, the two libraries the program calls were
opened as well:

| target | why |
| --- | --- |
| `/usr/bin/dolphin` | the Confirmations settings page, the keys it writes, the action captions |
| `/usr/lib/libdolphinprivate.so.6` | `DolphinView::trashSelectedItems` @0x27f2e0 — hands the URLs over, asks nothing itself |
| `/usr/lib/libKF6KIOWidgets.so.6.31.0` | `KIO::DeleteOrTrashJob` and `KIO::JobUiDelegate` — where the question is really asked and worded |

A Ghidra address is the file offset plus `0x100000` for `/usr/bin/dolphin`
(stripped, no symbols); in the two libraries the addresses below are virtual
addresses and the symbol names are real.

**The finding that changes the picture: Dolphin never asks the delete question.
KIO does.** `DolphinView::trashSelectedItems` @0x27f2e0 builds a
`KIO::DeleteOrTrashJob`; its result comes back in
`DolphinView::slotTrashFileFinished` @0x27be30. Everything below — the switch,
its default, the sentences, the button labels — lives in `libKF6KIOWidgets`.

### The three switches and their defaults

`KIO::WidgetsAskUserActionHandler::askUserDelete` @0x1a7980
(`libKF6KIOWidgets.so.6.31.0`, pseudo-code `rea/deep4/114-pseudo-1a7a31.c`,
evidence `ev_c9bf156e…4544f6b`) reads the group **`[Confirmations]`** of
**`kiorc`** (both names UTF-16 in the library: group @0x21e8ac, file @0x21e8c8,
read back with `read_bytes`, `ev_ede36175…c22b5f`). Which key it reads depends on
the deletion type passed in:

| deletion type | key (string address) | default |
| --- | --- | --- |
| 0 — Delete | `ConfirmDelete` @0x21e8d4 (13 chars) | **true** |
| 1 — Move to Trash | `ConfirmTrash` @0x21e8f0 (12 chars) | **false** |
| 2 — Empty Trash | `ConfirmEmptyTrash` @0x21f2a8 (17 chars, `ev_daef67a0…f1ef99`) | **true** |
| 3, 4 (other delete kinds) | `ConfirmDelete` @0x21e8d4 again | **true** |

The default is the byte the function stores before the read: `local_101 = 1` at
the top, flipped to `0` only in the Trash branch. When the read says "do not
ask" the function calls `KIO::AskUserActionInterface::askUserDeleteResult` with
the first (yes) result and no dialog appears at all.

Dolphin's own settings page agrees, function by function:

* `FUN_00228ba0` @0x228ba0 (page load, `rea/deep4/033-pseudo-confirmtrash-ref2.c`,
  `ev_3cfcd8e2…80b950`) reads `ConfirmTrash`, `ConfirmEmptyTrash` and
  `ConfirmDelete` — in that order — and ticks the three boxes from them. The
  defaults it passes are three bytes at **0x243f00 = 1, 0x243f01 = 1,
  0x243f02 = 0** (read with `read_bytes`, `ev_b167d30b…b35b60` and
  `ev_baf8d6b4…c12bef`; they sit right after the string `sSettingsPage`).
* `FUN_002283f0` @0x2283f0 (page save, `rea/deep4/032-pseudo-confirmtrash-ref1.c`,
  `ev_5b21dd77…75469b`) writes the three `writeEntry` calls back into
  `[Confirmations]` of `kiorc`, and in the same page maps the *Executable
  scripts* combo box to `behaviourOnLaunch` = `alwaysAsk` / `open` / `execute`.
* `FUN_001d5010` @0x1d5010 (`rea/deep4/031-pseudo-confirmations-page.c`,
  `ev_9c70ab6e…2ed43`) is the page itself: **seven** checkboxes, the group
  header **"Ask for confirmation in Dolphin when:"** @0x236af0
  (`ev_b30c319c…ccbfe`), and the three delete lines **"Moving files or folders
  to trash"** @0x236aa0, **"Deleting files or folders"** @0x22e627 (used at
  0x1d57a0) and **"Emptying trash"** @0x22e618 (used at 0x1d572c). The other four
  are the tab, folder, terminal and admin questions.

### The sentences in the box

`KIO::JobUiDelegate::askDeleteConfirmation` @0x1a8a20 (`libKF6KIOWidgets`,
`rea/deep4/112-pseudo-askdelete.c`, `ev_d75520fb…a8554`):

* Moving to the Trash, one item — continue button **"Move to Trash"** (0x21e94c),
  cancel is the standard Cancel, and the text is
  `Do you really want to move this item to the Trash?` followed by
  `<nl/><filename>%1</filename>`, i.e. the item's name on its own line;
* moving several — the same question through `ki18ndp` (plural) and
  `KMessageBox::warningContinueCancelList`, which lists every marked name;
* deleting permanently — continue button **"Delete Permanently"**, text
  `Do you really want to permanently delete this item?`, the name line, a blank
  line, then `This action cannot be undone.` inside
  `<emphasis strong='true'>`; several items take the list form with the same
  warning;
* emptying the Trash — continue button **"Empty Trash"** with a theme icon. Its
  question text is **not** in the binary: the call is
  `kxi18ndc("kio6", "@info")`, a lookup in kio's translation catalogue, so the
  wording of that one sentence stays unknown here and the rebuild keeps its own;
* all three boxes pass flag `0x21` to `KMessageBox` — Dangerous plus a Continue
  button — which is why the continue button, not the cancel button, is the one
  that carries the warning colour.

### Inside the Trash, Delete means delete

`KIO::DeleteOrTrashJob::DeleteOrTrashJob` @0x1b0fa0
(`rea/deep4/103-pseudo-job-ctor.c`, `ev_e4af4735…bd4bdab03`): when the requested
type is Trash (1) and the first URL's scheme is `trash`, the job rewrites its
deletion type to Delete (0). So Delete on a row in the Trash skips the move and
asks the permanent-delete question, under the `ConfirmDelete` switch, never
`ConfirmTrash`. `KIO::DeleteOrTrashJob::start` @0x1b4970 then hands the work to
the `AskUserActionInterface` of the window's UI delegate.

### The captions on the row buttons

`FUN_001c81f0` @0x1c81f0 (`rea/deep4/034-pseudo-permadelete.c`,
`ev_beb9f0bf…dd1243b`) is the function that captions the buttons a row shows on
hover. The list, all from that one switch:

| case | caption | string |
| --- | --- | --- |
| 0 | `Copy %2 to the Clipboard` | — |
| 1 | `Copy the Location of %2 to the Clipboard` | — |
| 3 | `Cut %2 to the Clipboard` | — |
| 4 | `Permanently Delete %2` | 0x22e3b1 |
| 5 | `Duplicate %2` | — |
| 8 | `Move %2 to the Trash` | 0x22e3e6 |
| 10 | `Rename %2` | — |

The caption is measured with `QFontMetrics::averageCharWidth` and elided to
**20 × that width** (`imul …, 0x14`), so the button keeps the same width for
every name.

### What the rebuild took from this pass

* **The words.** Both questions now use the original's sentences and the item's
  own name on a line under the question (`app/src/main.rs`,
  `display_name` / `name_list`); the continue buttons read "Move to Trash",
  "Delete Permanently" and "Empty Trash" (`app/src/panels.rs`), and the Trash
  view's own rows already carried the last two.
* **The defaults.** `confirm_delete` now ships **on**, the original's default,
  so Shift+Delete asks first. `confirm_trash` stays **off**: it is off in the
  original too, and the person asked for a marked picture and a press of Delete
  to be the whole action. `confirm_empty_trash` stays on. This is the exact
  three-way state the original opens with, and `--selftest` now states it.
* **The Trash rule** (Delete inside the Trash is a permanent delete) was already
  in the build; it is now cited to `DeleteOrTrashJob`'s constructor instead of
  being described from behaviour.
* **Still the rebuild's own:** the wording of the Empty-Trash question (not in
  the binary), and the eight-name cap on the list under a multiple-item
  question, where the original lists every name in a scrolling
  `KMessageBox::warningContinueCancelList`.

**Checked in the running window** (`app/shots/delete-question.png`): the box
that Shift+Delete opens on three marked pictures, reading *Delete Permanently* /
*Do you really want to permanently delete these items?* / *3 items:* with the
three names under it / *This action cannot be undone.*, its continue button
carrying the original's label, and nothing deleted while it was up. With the
Trash switch turned on, `app/shots/trash-question.png` shows the other path:
*Move to Trash* / *Do you really want to move these items to the Trash?* / the
three names / a blue *Move to Trash* button.

`--selftest` covers this without a window: *"a fresh profile asks no question
before moving to the Trash, but does before a permanent delete and before
emptying the Trash"* and *"the delete question is worded the way the original
words it"*.

## The places panel's recent groups, and the shorter toolbar (this pass)

### What the original has

Two independent pieces of evidence, both looked at rather than assumed.

**The strings.** `libdolphinprivate.so.6` holds one run of empty-state sentences,
which is where the group's own wording was found. Offsets are file offsets,
verified by a byte search of the installed library:

| string | offset |
| --- | --- |
| `recentlyused` (KIO worker name) | 0x1bf92a |
| `No recently used items` | 0x1bf937 |
| `Trash is empty` | 0x1bf8f9 |
| `Folder is empty` | 0x1bf9de |
| `No items matching the search` | 0x1bf8dc |

The empty case has a sentence of its own, written in the same style as the
Trash's and an empty folder's — so *Recent Files* is a real place in the original,
not something invented here. This build shows that sentence word for word when
the group has nothing in it.

**The sidebar.** A capture of the installed program
(`rea/orig-default.png`, 1600×1000) shows the panel as: **Places** (Home,
Wastebin), **Remote** (Network), **Recent** (a document-with-clock row reading
*Recent Files*, and a folder-with-clock row reading *Recent Locatio…* — cut off by
the panel's width), then **Devices** (each with a free-space bar). So the original
has *one* heading, *Recent*, holding two rows, and each row is a place that opens
a list in the main view (`recentlyused:/` and the recent-locations place), not a
list drawn inside the panel.

The rebuild keeps the original's names and both of its rows' purposes but draws
them differently: two headings, *Recent Files* and *Recent Locations*, each of
which opens to show up to eight files or six folders **inside the panel**, and
folds shut when the person does not need it. That is a deliberate difference, made
because the panel is where the person already looks for a way back somewhere, and
because the build has no `recentlyused:` worker to navigate to. It is listed in
`mechanics.json` (`recent` group) as this build's arrangement, next to the
original's wording, so the two can be told apart.

The list itself is the freedesktop one, `~/.local/share/recently-used.xbel`: XML
`<bookmark href=…>` entries carrying `added`, `modified` and `visited` stamps.
KRecentDocument maintains it and KIO's `recentlyused:` worker sorts by `visited`,
which is the rule this build uses, so the two agree on what "recent" means and on
the order of the rows.

### The rules the rebuild keeps

* **Order: `visited` stamp, newest first** — not the file's modification date. A
  document written last year and opened this morning is the most recent thing to
  the person. The row's tooltip still shows the file's own date, read from the
  file, because that is what a file's date means everywhere else in the window.
* **One row per file.** A second bookmark for the same path is not a second row;
  the later visit is the one that counts.
* **Only files, and only files still there.** Folders belong to *Recent
  Locations*; a bookmark whose file has been moved or deleted is dropped rather
  than drawn as a row that opens nothing, and a non-local address (`smb://`,
  `trash:/`) is dropped because this build cannot open it.
* **Rows shown: 8** (`recent_rows()` in `src/config.rs`); **folders kept: 6**
  (`RECENT_LOCATIONS_MAX` in `src/main.rs`), newest first, each once — walking back
  into a folder moves it to the top instead of adding a duplicate.
* **Both headings fold on one click**, with the count at the right so a folded
  group still says how much is behind it.

The two counts, 8 and 6, are **this build's own** numbers. The original caps
nothing — its rows open a full view that scrolls — so there was no number in the
code to copy. They were picked to fill the panel without pushing *Devices* off the
bottom, and are written in `mechanics.json` as the build's values, not as
measurements.

### The toolbar

The same capture shows the original's row reading: back (with a dropdown), forward
(with a dropdown), up, then the breadcrumb bar, with the rest of the cluster to its
right. The rebuild's row had an extra *Open in* button at the left of the cluster,
whose menu listed a new tab, a new window, the other pane and a terminal — a second
way to do what a double-click, Enter and the right-click menu already do. It is
gone; opening is a double-click, Enter, or *Open* / *Open With* on the right-click
menu, and the row is shorter by one button and its menu.

The app menu is now a scrolling list capped to the window's height: in a window
that was not maximised its last rows (*Help*, *About*, *Quit*) could sit below the
bottom edge with no way to reach them. *About* names the release the build came
from — the crate version, which is the GitHub tag without its `v` — so a picture of
the box says which build it is.

**Checked without a mouse:** `--selftest` *"the two button words survive an
ordinary window and give way in order"* walks the toolbar geometry from 360 to
1600 px and fails if the location bar and the buttons ever overlap, or if the words
go while they would still fit. The recent groups are covered by *"recent files come
newest visit first, without folders or files that have gone"*, *"a folder walked
into is remembered once, newest first, and the list stops at six"* and *"a recent
group folds with one click and opens with the next"*.

### The Remote group (the *Network* row)

The same sidebar capture shows a **Remote** heading holding a single **Network**
row, sitting between *Places* and *Recent*. In the original that row is a KIO
place: clicking it opens `network:/`, the worker that browses SMB, SFTP, NFS and
WebDAV through the system's own network stack, with its own device discovery.

This build has no KIO, so it cannot browse the network — and a row that opens an
empty folder would be worse than none. What it does instead: the group lists the
network folders the desktop **already has**, read from the two places Linux keeps
them.

* `/proc/mounts`, keeping only the network filesystem types: `cifs`, `smbfs`,
  `nfs`, `nfs4`, `afs`, `9p`, `davfs`, `davfs2`, `fuse.sshfs`, `fuse.rclone`,
  `fuse.s3fs`, `fuse.davfs2`. A mount point is listed only if the folder is really
  there. The name on the row is the share for an SMB mount (`//router/media` ->
  *media*), otherwise the last part of the mount point.
* `/run/user/<uid>/gvfs`, the folder of mounts the desktop's own file choosers
  create. Each folder is named after its parameters (`smb-share:server=…,share=…`,
  `sftp:host=…,user=…`); the row shows the share, or the host when there is no
  share, with the protocol in brackets for SFTP, FTP and WebDAV.

Escaping is handled the way the mount table writes it (`\040` for a space in
either the source or the mount point) — a share named *Team Share* must not read
as `Team\040Share`.

With nothing mounted the group shows the original's own sentence,
**No shared folders found** (`0x1bf952` in `libdolphinprivate.so.6`, in the same
run of empty-state strings as `0x1bf937` *No recently used items*). The heading
folds like the recent groups, so an empty Network group costs one line, not four.

The group's own label in the original is *Remote* with a row called *Network*;
this build draws one folding group called *Network* directly. That is written in
`mechanics.json` (`network` group) next to the original's wording.

`--selftest` *"the Network group lists mounted shares and nothing else"* feeds the
parser a sample mount table — a local ext4 root, a tmpfs, an SMB share, an NFS
export, an sshfs mount and a local vfat with an escaped space — and fails unless
exactly the three shares come back, named *media*, *photos* and *data*, with the
escaped mount point unescaped and the local drive left out. It also checks the two
GIO folder names.
