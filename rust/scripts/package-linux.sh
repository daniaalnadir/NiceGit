#!/bin/bash
set -euo pipefail

# Packages a built NiceGit into the Linux downloads for this machine's architecture, in rust/dist:
#   NiceGit-<version>-linux-<arch>.tar.gz    the app, its icon and desktop entry
#   NiceGit-<version>-linux-<debarch>.deb    Debian and Ubuntu
#   NiceGit-<version>-linux-<arch>.rpm       Fedora, RHEL and openSUSE
#   NiceGit-<version>-linux-<arch>.AppImage  any distribution, no installation
# <arch> is x86_64 or aarch64; <debarch> is amd64 or arm64.
#
# Run after `cargo build --release` and rust/scripts/third-party-licenses.sh. Needs cargo-deb and
# cargo-generate-rpm; appimagetool and its runtime are downloaded at pinned versions and checked.
#   cargo install cargo-deb --locked --version 3.8.0
#   cargo install cargo-generate-rpm --locked --version 0.21.1
#
# Example: bash rust/scripts/package-linux.sh 2.1.0

version="${1:?usage: package-linux.sh <version>}"
rust_dir="$(cd "$(dirname "$0")/.." && pwd)"
root="$(dirname "$rust_dir")"
dist="$rust_dir/dist"
binary="$rust_dir/target/release/nicegit"
notices="$dist/THIRD_PARTY_LICENSES.txt"
for file in "$binary" "$notices"; do
    [ -f "$file" ] || { echo "$file is missing; build NiceGit and its licence file first" >&2; exit 1; }
done

arch="$(uname -m)"
case "$arch" in
    x86_64)
        debarch=amd64
        appimagetool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
        runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
        ;;
    aarch64)
        debarch=arm64
        appimagetool_sha256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
        runtime_sha256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
        ;;
    *) echo "No Linux packages are made for $arch" >&2; exit 1 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$dist"
name="NiceGit-$version-linux-$arch"

# Tarball.
stage="$work/$name"
mkdir -p "$stage"
cp "$binary" "$stage/nicegit"
cp "$root/LICENSE" "$root/README.md" "$notices" "$rust_dir/packaging/linux/nicegit.desktop" "$stage/"
cp "$rust_dir/assets/icon-256.png" "$stage/nicegit.png"
tar -czf "$dist/$name.tar.gz" -C "$stage" .

cd "$rust_dir"

# Debian package. The output name is chosen here so it matches the other downloads.
cargo deb -p nicegit --no-build --output "$dist/NiceGit-$version-linux-$debarch.deb"

# RPM package. Pre-release versions such as 2.1.0-rc.1 use RPM's ~ so they sort before 2.1.0.
rpm_version="$(printf '%s' "$version" | tr - '~')"
cargo generate-rpm -p crates/nicegit \
    --set-metadata "version = \"$rpm_version\"" \
    --output "$dist/$name.rpm"

# AppImage: the app, its desktop entry and icon in an AppDir, packed by appimagetool.
download() { # url sha256 destination
    curl --fail --silent --show-error --location --retry 3 -o "$3" "$1"
    echo "$2  $3" | sha256sum --check --quiet
}
download "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$arch.AppImage" \
    "$appimagetool_sha256" "$work/appimagetool"
download "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" \
    "$runtime_sha256" "$work/runtime"
chmod +x "$work/appimagetool"

appdir="$work/NiceGit.AppDir"
install -Dm755 "$binary" "$appdir/usr/bin/nicegit"
install -Dm644 "$rust_dir/packaging/linux/nicegit.desktop" "$appdir/usr/share/applications/nicegit.desktop"
install -Dm644 "$rust_dir/assets/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/nicegit.png"
install -Dm644 "$notices" "$appdir/usr/share/doc/nicegit/THIRD_PARTY_LICENSES.txt"
install -Dm644 "$root/LICENSE" "$appdir/usr/share/doc/nicegit/LICENSE"
cp "$rust_dir/packaging/linux/nicegit.desktop" "$appdir/nicegit.desktop"
cp "$rust_dir/assets/icon-256.png" "$appdir/nicegit.png"
ln -s usr/bin/nicegit "$appdir/AppRun"
# appimagetool is itself an AppImage; extracting it avoids needing FUSE on build machines.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$work/appimagetool" --no-appstream \
    --runtime-file "$work/runtime" "$appdir" "$dist/$name.AppImage"

ls -l "$dist"
