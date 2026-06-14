#!/bin/sh
# Launch cce-server with in-process cce-client
# Usage: ./start-rust.sh [--logging] [--debug]
 
LOGGING=false
DEBUG=false
for arg in "$@"; do
    case "$arg" in
        --logging) LOGGING=true ;;
        --debug) DEBUG=true ;;
    esac
done
 
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export XCURSOR_THEME="crosshair-theme"
export XCURSOR_SIZE=24
export XCURSOR_PATH="/home/lsgalante/.local/share/icons:/home/lsgalante/.icons:/usr/share/icons"
export WLR_NO_HARDWARE_CURSORS=1
 
# Resolve script directory to reference target/release/cce-server reliably
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$DEBUG" = true ]; then
    export WAYLAND_DEBUG=1
fi

if [ "$LOGGING" = true ] || [ "$DEBUG" = true ]; then
    exec "$SCRIPT_DIR/target/release/cce" 2>/tmp/river-cce-client.log
else
    exec "$SCRIPT_DIR/target/release/cce" --log-level error 2>/dev/null
fi
