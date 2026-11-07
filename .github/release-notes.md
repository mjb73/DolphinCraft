DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.4

* **Find in Files** (**Ctrl+Shift+F**): the find bar only ever filtered names; this
  window reads inside them. Three modes worded the way Dolphin words them — *File
  names*, *File names and contents*, *File contents* — with a *Case sensitive*
  switch and an *Include binary files* switch, one row per file carrying the line
  the phrase was written on, and **Show** / **Open** beside every row. It never
  follows links, leaves hidden folders out, skips anything over 4 MB, and its
  summary line says exactly what it passed over.
* **Manage Disk Space Usage**, under *Tools*: adds up everything below the folder on
  show and lists what takes the room, largest first, with a *Free Up Disk Space*
  shortlist of the biggest files and a **Show** button for each. The original hands
  this question to Filelight and offers to install it; this window answers it
  itself, so nothing has to be installed. Hidden files count, a symbolic link is
  never followed, and a folder you cannot read is reported instead of being quietly
  left out.
* **Compress…** turns the marked items — or the folder on show — into one ZIP in the
  folder they came from, with the name filled in and editable first: `photos.zip`
  for one folder, `Archive.zip` for several items, and `photos (1).zip` rather than
  overwriting anything. The size of what is marked is added up **off the drawing
  thread**, so a big folder shows *adding up…* while the window keeps drawing.
* **Extract…** unpacks a ZIP or a tar archive into a folder named after the file
  (or into this folder, if you untick *Into a folder of its own*) and takes you
  there afterwards. Both tools use the `zip`, `unzip` and `tar` programs your
  computer already has, and say so plainly when one is missing.
* **The Network group** in the sidebar, where the original's *Remote* heading sits:
  it lists the network folders this computer already has — SMB, NFS and SSH shares
  in the mount table, and shares opened through the desktop's own file choosers,
  each named after its share. Nothing is mounted from here; with none mounted it
  says *No shared folders found*, and the heading folds away.
* **Recent Files is ordered by visit, not by date**: a document written last year
  and opened this morning now sits at the top, one row per path, and files that
  have since been moved or deleted are left out rather than offered as dead rows.
  The panel scrolls now that it carries four groups, so the last one is no longer
  pushed under the bottom edge.
* **Previews on and off each keep their own zoom size**, as the original does: zoom
  the wall with previews on, switch them off, and the size you chose is waiting
  when you switch them back. The picture-strip slider stays inside its one-row
  strip, and the details rows keep the original's 20 px side padding.
* **About** names the release this build came from — 0.1.4.
* 111 built-in checks (`dolphincraft --selftest`), all passing — the new ones cover
  the disk-usage walk, the compress and extract windows, the text search and what it
  skips, the recent-file order and the network parser.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.

**Archives** — `dolphincraft-0.1.4-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.4-x86_64-linux.tar.gz
./dolphincraft-0.1.4-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine; nothing dials out.
