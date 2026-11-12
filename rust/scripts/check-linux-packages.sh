#!/bin/bash
set -euo pipefail

# Checks the Linux downloads made by package-linux.sh for this machine's architecture: each holds
# the app built for the right architecture, the .deb installs on this system, the .rpm installs on
# Fedora (in a container), and the AppImage finds every library it needs and starts.
# Needs sudo, Docker and Xvfb, as on GitHub's Ubuntu runners.
#
# Example: bash rust/scripts/check-linux-packages.sh 2.1.0

version="${1:?usage: check-linux-packages.sh <version>}"
dist="$(cd "$(dirname "$0")/.." && pwd)/dist"
arch="$(uname -m)"
case "$arch" in
    x86_64) debarch=amd64 elf="x86-64" ;;
    aarch64) debarch=arm64 elf="ARM aarch64" ;;
    *) echo "No Linux packages are made for $arch" >&2; exit 1 ;;
esac
name="NiceGit-$version-linux-$arch"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fail() { echo "::error::$*" >&2; exit 1; }
check_binary() { # path
    file "$1" | grep -q "$elf" || fail "$1 is not built for $arch: $(file "$1")"
    missing="$(ldd "$1" | grep 'not found' || true)"
    [ -z "$missing" ] || fail "$1 needs libraries this system lacks: $missing"
}

# Tarball.
mkdir "$work/tar"
tar -xzf "$dist/$name.tar.gz" -C "$work/tar"
for file in nicegit nicegit.desktop nicegit.png LICENSE README.md THIRD_PARTY_LICENSES.txt; do
    [ -e "$work/tar/$file" ] || fail "$name.tar.gz lacks $file"
done
check_binary "$work/tar/nicegit"

# Debian package, installed with its dependencies, then removed.
deb="$dist/NiceGit-$version-linux-$debarch.deb"
sudo apt-get install -y "$deb"
check_binary /usr/bin/nicegit
for file in /usr/share/applications/nicegit.desktop /usr/share/icons/hicolor/256x256/apps/nicegit.png; do
    [ -f "$file" ] || fail "the .deb did not install $file"
done
sudo apt-get remove -y nicegit
[ ! -e /usr/bin/nicegit ] || fail "removing the .deb left /usr/bin/nicegit behind"

# RPM package, installed with its dependencies on Fedora.
docker run --rm -v "$dist:/dist:ro" fedora:42 bash -euc "
    dnf install -y --setopt=install_weak_deps=False /dist/$name.rpm
    test -x /usr/bin/nicegit
    test -f /usr/share/applications/nicegit.desktop
    rpm -q --requires nicegit | grep -qx git
    missing=\$(ldd /usr/bin/nicegit | grep 'not found' || true)
    test -z \"\$missing\" || { echo \"missing libraries: \$missing\"; exit 1; }
    dnf remove -y nicegit
    test ! -e /usr/bin/nicegit"

# AppImage: unpacked, so the check needs no FUSE, then started on a virtual display.
appimage="$dist/$name.AppImage"
[ -x "$appimage" ] || fail "$name.AppImage is not executable"
(cd "$work" && "$appimage" --appimage-extract > /dev/null)
for file in AppRun nicegit.desktop nicegit.png usr/bin/nicegit; do
    [ -e "$work/squashfs-root/$file" ] || fail "$name.AppImage lacks $file"
done
check_binary "$work/squashfs-root/usr/bin/nicegit"
xvfb-run -a -s "-screen 0 1600x1000x24" bash -c '
    APPIMAGE_EXTRACT_AND_RUN=1 "$1" > "$2/appimage.log" 2>&1 &
    pid=$!
    sleep 20
    kill -0 $pid || { cat "$2/appimage.log"; exit 1; }
    kill $pid' _ "$appimage" "$work" || fail "$name.AppImage did not stay running"

echo "The $arch Linux downloads are good"
