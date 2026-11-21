#!/bin/sh
# Build the duxdm .deb from the current release binary.
# Usage: ./scripts/build-deb.sh
set -e
cd "$(dirname "$0")/.."

VERSION="0.1.0"
DEB="duxdm_${VERSION}-1_amd64.deb"

cargo build --release

STAGE=$(mktemp -d /tmp/duxdm-stage.XXXXXX)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/DEBIAN" \
         "$STAGE/usr/bin" \
         "$STAGE/etc/pam.d" \
         "$STAGE/lib/systemd/system" \
         "$STAGE/usr/share/doc/duxdm" \
         dist

install -m 0755 target/release/duxdm          "$STAGE/usr/bin/duxdm"
install -m 0644 packaging/duxdm.service       "$STAGE/lib/systemd/system/duxdm.service"
install -m 0644 packaging/pam-duxdm           "$STAGE/etc/pam.d/duxdm"
install -m 0755 packaging/postinst            "$STAGE/DEBIAN/postinst"
install -m 0755 packaging/prerm               "$STAGE/DEBIAN/prerm"
install -m 0644 packaging/control             "$STAGE/DEBIAN/control"
install -m 0644 LICENSE                       "$STAGE/usr/share/doc/duxdm/copyright"

# Normalize directory modes (stage dir comes from mktemp with 700/775).
find "$STAGE" -type d -exec chmod 0755 {} \;

dpkg-deb --build --root-owner-group "$STAGE" "dist/$DEB"
echo "built dist/$DEB"
