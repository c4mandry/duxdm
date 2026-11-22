#!/bin/sh
# Publish the built .deb into the webnotes.work apt repository.
# Usage: ./scripts/publish-apt.sh [path/to/webnotes-repo]
# After this, rebuild and restart the webnotes site to deploy:
#   (cd <webnotes> && npm run build && systemctl --user restart webnotes-prod)
set -e
cd "$(dirname "$0")/.."

WEBROOT="${1:-/home/ufi/Documents/v3.1/cont/v3.1}"
REPO="$WEBROOT/public/duxdm-repo"
GPG_KEY="28B08282939195CB169045DEF13CD4E16B689115"
FTP_CONF=$(mktemp /tmp/duxdm-ftp.XXXXXX)
trap 'rm -f "$FTP_CONF"' EXIT

DEB=$(ls dist/duxdm_*_amd64.deb | sort | tail -1)
test -f "$DEB" || { echo "no .deb in dist/ — run ./scripts/build-deb.sh first"; exit 1; }

mkdir -p "$REPO/dists/stable/main/binary-amd64"
cp "$DEB" "$REPO/dists/stable/main/binary-amd64/"

# Publish the bootstrap installer and the signing keys (armored + binary;
# the binary keyring lets install.sh work on systems without gpg).
install -m 0755 scripts/install.sh "$REPO/install.sh"
gpg --armor --export "$GPG_KEY" > "$REPO/key.gpg"
gpg --yes --dearmor -o "$REPO/keyring.gpg" "$REPO/key.gpg"

cat > "$FTP_CONF" <<'EOF'
APT::FTPArchive::Release::Origin "duxdm";
APT::FTPArchive::Release::Label "duxdm";
APT::FTPArchive::Release::Suite "stable";
APT::FTPArchive::Release::Codename "stable";
APT::FTPArchive::Release::Architectures "amd64";
APT::FTPArchive::Release::Components "main";
APT::FTPArchive::Release::Description "duxdm apt repository";
EOF

(
    cd "$REPO" &&
    dpkg-scanpackages -m dists/stable/main/binary-amd64 /dev/null \
        > dists/stable/main/binary-amd64/Packages &&
    apt-ftparchive -c "$FTP_CONF" release dists/stable > dists/stable/Release &&
    gpg --batch --yes --armor --detach-sign -o dists/stable/Release.gpg dists/stable/Release &&
    gpg --batch --yes --clearsign -o dists/stable/InRelease dists/stable/Release
)

echo "published $DEB into $REPO — rebuild + restart webnotes to deploy."
