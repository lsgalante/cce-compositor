#!/bin/bash
# Launch river with clearwm-rs on this TTY
# Usage: Switch to a free TTY, log in, and run this script

export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-1

# Create the River init executable (clearwm launch script)
# This must exist before River starts, and /tmp is cleared on reboot.
cat > /tmp/clearwm-rs-launch.sh << 'LAUNCH_EOF'
#!/bin/sh
exec /home/lsgalante/.local/bin/clearwm 2>/tmp/clearwm-test.log
LAUNCH_EOF
chmod +x /tmp/clearwm-rs-launch.sh

echo "Starting river with clearwm-rs..."
echo "Log will be at /tmp/river-clearwm.log"

exec river -c /tmp/clearwm-rs-launch.sh 2>/tmp/river-clearwm.log
