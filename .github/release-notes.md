DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

## What's new in 0.1.8

* **The system's file dialog is this window.** Every program with an *Open* or
  *Save* button asks the desktop for a file, and on this machine the answer was
  plasma's own dialog. It is now DolphinCraft's: `dolphincraft --pick open` (or
  `--pick save`) is the whole file manager as a dialog — places, tabs, all four
  views, the find bar, the type-ahead — with a footer carrying the question, the
  type filter, the save name field and Cancel / Open, and the answer printed on
  stdout as one absolute path per line. `--folder` asks for a folder instead,
  `--multi` allows several files, `--filter "Images:*.png;*.jpg"` gives it the
  rows the asking program declared, `--name` fills the save field and `--title`
  says what is being asked. The dialog offers nothing that makes, renames or
  deletes a file: a right click is one *Open* row, the second click that turns a
  row into a name field is off, the file window's key map is not read at all, and
  not one setting is written — it is another program's question, not a session.
  Cancel, Escape and closing the window all answer the same way.
* **A backend puts it behind the desktop's portal.** `packaging/portal/` answers
  `org.freedesktop.impl.portal.FileChooser` — version 3: open a file, save a
  file, save several — plus `Request.Close`, so a program that cancels its
  request also ends the window. One command installs it and nothing needs root:
  `sh packaging/portal/install-portal.sh`. Only one line of
  `~/.config/xdg-desktop-portal/portals.conf` changes, so secrets, settings and
  notifications keep the backends the desktop gave them, and `--uninstall` puts
  the file back as it was. It reaches Flatpak and Snap applications always, and
  any GTK, Electron or Firefox program that has been told to use the portal. KDE
  and Qt programs draw their own KIO dialog inside their own process and keep
  it — replacing that would mean forking KIO.
* **The desktop's file manager, on `org.freedesktop.FileManager1`.** Another
  program's *Show in folder* — a finished download, a mounted device, a chat
  attachment — opens DolphinCraft with the file marked in the folder that holds
  it; *Show item properties* opens it with the Properties window up; a request
  that arrives with no window open starts one. `DesktopName` answers
  `DolphinCraft`. This is the name Plasma's Dolphin daemon answered, so every
  file association was already this build's and those requests still opened
  Dolphin. Now they do not.
* **The picture wall reaches into the folders below.** A *Sub-folders* switch
  shows the pictures of the whole tree — a library sorted into year or month
  folders reads as one wall — through the same listing a search uses, so marking,
  Ctrl+A, the arrow keys, the viewer and the slideshow keep working on the real
  files. Untick it and the wall is this folder alone again. The find bar's
  content search walks the same way.
* **A preview is decoded by the file's own bytes, not by its name.** A PNG that
  arrived named `.jpg` — a download, a renamed export — showed a broken tile
  before and now shows its picture; the viewer decodes through the same helper. A
  video is recognised by its magic as well as its extension (`ftyp`, Matroska's
  EBML header, `RIFF…AVI`), so a clip renamed `.jpg` still gets its first frame
  from ffmpeg.
* **About** names the release this build came from — 0.1.8 — and says it dials out
  only for *Check for Updates…*, when asked.
* 165 built-in checks (`dolphincraft --selftest`), all passing — the new ones
  cover what a dialog's filter keeps and what it drops, the glob matcher, a filter
  row read out of the word a program passes, which marked file a dialog may
  answer, a folder request that answers the folder on screen, and a save dialog's
  name read as a path.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.
From 0.1.7 on, the menu's *Check for Updates…* does the same job without the shell command.

**Archives** — `dolphincraft-0.1.8-x86_64-linux.tar.gz` (or `aarch64` for ARM) and an identical
`.zip`, each with a `.sha256`. Unpack one and run the binary:

```sh
tar xzf dolphincraft-0.1.8-x86_64-linux.tar.gz
./dolphincraft-0.1.8-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.35 — Ubuntu 22.04+,
Mint 21+, Fedora 36+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine. The only request DolphinCraft ever makes is the
update check, and only when *Check for Updates…* is picked.
