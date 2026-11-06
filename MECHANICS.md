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
* "Show folders first" and "Natural" sorting as factory defaults.

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

* A remembered split view (config `split: true`) used to come back as a single
  pane with the toolbar bar suppressed — the location bar disappeared entirely
  after restarting with split view on. `App::new` now rebuilds the second pane,
  so split view restores with one bar per pane, as the original does.
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
| sort control, narrow window | icon only, below about 660 px of window width | rebuild |
| sort control, narrowest | hidden; sorting stays in the app menu's *Sort by* | rebuild |
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
control in the toolbar with no bar to collide with.

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
