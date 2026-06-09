#!/bin/sh
# Launch cce-server with cce-client
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
 
# Create the cce-client launch script
DEBUG_FLAG=""
if [ "$DEBUG" = true ]; then
    DEBUG_FLAG="WAYLAND_DEBUG=1 "
fi
 
cat > /tmp/cce-client-launch-rust.sh << LAUNCH_EOF
#!/bin/sh
${DEBUG_FLAG}exec /home/lsgalante/.local/bin/cce-client 2>/tmp/cce-client-\${WAYLAND_DISPLAY}.log
LAUNCH_EOF
chmod +x /tmp/cce-client-launch-rust.sh
 
if [ "$LOGGING" = true ] || [ "$DEBUG" = true ]; then
    echo "Starting cce-server with cce-client..."
    echo "  Logs: /tmp/river-cce-client.log + /tmp/cce-client-\${WAYLAND_DISPLAY}.log"
    if [ "$DEBUG" = true ]; then
        echo "  Wayland debug logging enabled (WAYLAND_DEBUG=1)"
    fi
fi
 
# Resolve script directory to reference target/release/cce-server reliably
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$DEBUG" = true ]; then
    export WAYLAND_DEBUG=1
fi
exec "$SCRIPT_DIR/target/release/cce-server" -c /tmp/cce-client-launch-rust.sh 2>/tmp/river-cce-client.log
