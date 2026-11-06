#!/bin/sh
# Build DolphinCraft if it is not built yet, then open the window.
# Nothing extra is needed beyond a Rust toolchain (https://rustup.rs).
here=$(cd "$(dirname "$0")" && pwd)
bin="$here/target/release/file-craft"

if [ ! -x "$bin" ]; then
    ( cd "$here" && cargo build --release ) || exit 1
fi

exec "$bin" "$@"
