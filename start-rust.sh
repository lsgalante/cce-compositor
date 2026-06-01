#!/bin/sh
# Launch clear-computing-environment-server with ccec
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

# Create the ccec launch script
DEBUG_FLAG=""
if [ "$DEBUG" = true ]; then
    DEBUG_FLAG="WAYLAND_DEBUG=1 "
fi

cat > /tmp/ccec-launch-rust.sh << LAUNCH_EOF
#!/bin/sh
${DEBUG_FLAG}exec /home/lsgalante/.local/bin/ccec 2>/tmp/ccec-\${WAYLAND_DISPLAY}.log
LAUNCH_EOF
chmod +x /tmp/ccec-launch-rust.sh

if [ "$LOGGING" = true ] || [ "$DEBUG" = true ]; then
    echo "Starting clear-computing-environment-server with ccec..."
    echo "  Logs: /tmp/river-ccec.log + /tmp/ccec-\${WAYLAND_DISPLAY}.log"
    if [ "$DEBUG" = true ]; then
        echo "  Wayland debug logging enabled (WAYLAND_DEBUG=1)"
    fi
fi

# Resolve script directory to reference target/release/clear-computing-environment-server reliably
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$DEBUG" = true ]; then
    export WAYLAND_DEBUG=1
fi
exec "$SCRIPT_DIR/target/release/clear-computing-environment-server" -c /tmp/ccec-launch-rust.sh 2>/tmp/river-ccec.log

