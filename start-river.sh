#!/bin/bash
# Launch river with clearwm on this TTY
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

# Create the River init executable (clearwm launch script)
# This must exist before River starts, and /tmp is cleared on reboot.
if [ "$LOGGING" = true ]; then
    cat > /tmp/clearwm-rs-launch.sh << 'LAUNCH_EOF'
#!/bin/sh
exec /home/lsgalante/.local/bin/clearwm 2>/tmp/clearwm.log
LAUNCH_EOF
else
    cat > /tmp/clearwm-rs-launch.sh << 'LAUNCH_EOF'
#!/bin/sh
exec /home/lsgalante/.local/bin/clearwm
LAUNCH_EOF
fi
chmod +x /tmp/clearwm-rs-launch.sh

echo "Starting river with clearwm..."
if [ "$LOGGING" = true ]; then
    echo "Logs: /tmp/river-clearwm.log + /tmp/clearwm.log"
else
    echo "Logging disabled. Use --logging to enable."
fi

exec river -c /tmp/clearwm-rs-launch.sh 2>/tmp/river-clearwm.log
