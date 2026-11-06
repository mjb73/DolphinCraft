DolphinCraft — a desktop file manager in Rust and egui, rebuilt from Dolphin with a few tools of its own.

**Linux** — `install.sh` (no root), or the tarball for your architecture.
```sh
curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
```
It installs `dolphincraft` to `~/.local/bin`, writes the menu entry, and refreshes the launcher.

**Tarball** — `dolphincraft-<version>-x86_64-linux.tar.gz` (or `aarch64` for ARM) plus a `.sha256`.
Unpack it and run the binary:

```sh
tar xzf dolphincraft-0.1.0-x86_64-linux.tar.gz
./dolphincraft-0.1.0-x86_64-linux/dolphincraft
```

The tarballs are built on Debian 12, so they ask for glibc 2.36 — Debian 12, Ubuntu 22.04+,
Mint 21+, Fedora 37+ and anything newer. A Wayland or X11 session and an OpenGL driver
(libglvnd + mesa) are the only runtime requirements.

Build it yourself if you prefer: `cargo build --release`, which needs a Rust toolchain
(https://rustup.rs) and the dev packages `libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

Every file you see stays on your machine; nothing dials out.
