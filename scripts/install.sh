#!/bin/sh
# duxdm apt repository installer — one-liner bootstrap.
#
#   curl -fsSL https://webnotes.work/duxdm-repo/install.sh | sudo sh
#
# After this, duxdm is installed and future updates arrive through the
# normal apt upgrade flow. Safe to re-run.

set -e

REPO_URL="https://webnotes.work/duxdm-repo"
KEYRING="/etc/apt/keyrings/duxdm.gpg"
SOURCE_LIST="/etc/apt/sources.list.d/duxdm.list"

if [ "$(id -u)" -ne 0 ]; then
    echo "duxdm installer: please run as root:" >&2
    echo "  curl -fsSL $REPO_URL/install.sh | sudo sh" >&2
    exit 1
fi

if ! command -v apt-get >/dev/null 2>&1; then
    echo "duxdm installer: this repository is for apt-based systems (Debian/Ubuntu)." >&2
    exit 1
fi

echo "==> adding the duxdm signing key ($KEYRING)"
mkdir -p /etc/apt/keyrings
curl -fsSL "$REPO_URL/keyring.gpg" -o "$KEYRING"

echo "==> adding the duxdm apt source ($SOURCE_LIST)"
printf 'deb [signed-by=%s] %s stable main\n' "$KEYRING" "$REPO_URL" > "$SOURCE_LIST"

echo "==> updating package lists"
apt-get update

echo "==> installing duxdm"
# Noninteractive: on minimal/server systems, xinit's Recommends pulls
# keyboard-configuration, whose debconf dialog would otherwise block.
export DEBIAN_FRONTEND=noninteractive
apt-get install -y duxdm

echo
echo "duxdm installed."
echo "  It takes over tty1 (stops getty@tty1) — reboot to get the login screen,"
echo "  or start it right now:  sudo systemctl start duxdm"
echo "  Future updates:         sudo apt update && sudo apt upgrade"
