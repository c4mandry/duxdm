#!/bin/sh
# Set up a sandbox so the duxdm end-to-end test can run on this machine.
# Needs root (run with sudo). Safe to re-run.
set -e

# 1. PAM service config (same file the .deb installs)
cat > /etc/pam.d/duxdm <<'EOF'
# PAM configuration for duxdm — same stack as login/getty.
# Note: @include must be unprefixed (no module type) — Debian syntax.
@include common-auth
@include common-account
@include common-password
@include common-session
EOF

# 2. Test user
if ! id duxdmtest >/dev/null 2>&1; then
    useradd -m -s /bin/bash duxdmtest
fi
echo 'duxdmtest:duxdmtest123' | chpasswd

# 3. Fake session dirs (used via DUXDM_WAYLAND_SESSIONS / DUXDM_XSESSIONS)
mkdir -p /tmp/duxdm-test/wayland-sessions /tmp/duxdm-test/xsessions
cat > /tmp/duxdm-test/wayland-sessions/test.desktop <<'EOF'
[Desktop Entry]
Name=TestSession
Exec=/bin/sh -c 'echo DUXDM-SESSION-RAN; [ -n "$XDG_RUNTIME_DIR" ] && echo RUNTIME-OK; [ -n "$XDG_SEAT" ] && echo SEAT-OK; [ -n "$XDG_VTNR" ] && echo VTNR-OK'
EOF
cat > /tmp/duxdm-test/xsessions/broken.desktop <<'EOF'
[Desktop Entry]
Name=Broken X11
Exec=/bin/false
EOF

echo "test env ready: user=duxdmtest pw=duxdmtest123"
