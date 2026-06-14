#!/bin/bash
# Launch river with cce-client on this TTY
# Usage: Switch to a free TTY, log in, and run this script
 
LOGGING=false
for arg in "$@"; do
    case "$arg" in
        --logging) LOGGING=true ;;
    esac
done
 
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-1
export XCURSOR_THEME="crosshair-theme"
export XCURSOR_SIZE=24
export XCURSOR_PATH="/home/lsgalante/.local/share/icons:/home/lsgalante/.icons:/usr/share/icons"
 
# Create the River init executable (cce-client launch script)
# This must exist before River starts, and /tmp is cleared on reboot.
if [ "$LOGGING" = true ]; then
    cat > /tmp/cce-client-launch-river.sh << 'LAUNCH_EOF'
#!/bin/sh
exec /home/lsgalante/.local/bin/cce client 2>/tmp/cce-client-${WAYLAND_DISPLAY}.log
LAUNCH_EOF
else
    cat > /tmp/cce-client-launch-river.sh << 'LAUNCH_EOF'
#!/bin/sh
exec /home/lsgalante/.local/bin/cce client
LAUNCH_EOF
fi
chmod +x /tmp/cce-client-launch-river.sh
 
if [ "$LOGGING" = true ]; then
    exec river -c /tmp/cce-client-launch-river.sh 2>/tmp/river-cce-client.log
else
    exec river -c /tmp/cce-client-launch-river.sh 2>/dev/null
fi
