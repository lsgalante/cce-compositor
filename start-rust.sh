#!/bin/sh
# Launch clear-river with clearwm
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

# Create the clearwm launch script
DEBUG_FLAG=""
if [ "$DEBUG" = true ]; then
    DEBUG_FLAG="WAYLAND_DEBUG=1 "
fi

cat > /tmp/clearwm-rs-launch-rust.sh << LAUNCH_EOF
#!/bin/sh
${DEBUG_FLAG}exec /home/lsgalante/.local/bin/clearwm 2>/tmp/clearwm-\${WAYLAND_DISPLAY}.log
LAUNCH_EOF
chmod +x /tmp/clearwm-rs-launch-rust.sh

if [ "$LOGGING" = true ] || [ "$DEBUG" = true ]; then
    echo "Starting clear-river with clearwm..."
    echo "  Logs: /tmp/river-clearwm.log + /tmp/clearwm-\${WAYLAND_DISPLAY}.log"
    if [ "$DEBUG" = true ]; then
        echo "  Wayland debug logging enabled (WAYLAND_DEBUG=1)"
    fi
fi

# Resolve script directory to reference target/release/clear-river reliably
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$DEBUG" = true ]; then
    export WAYLAND_DEBUG=1
fi
exec "$SCRIPT_DIR/target/release/clear-river" -c /tmp/clearwm-rs-launch-rust.sh 2>/tmp/river-clearwm.log

