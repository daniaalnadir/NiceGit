#!/bin/bash
set -euo pipefail

# Builds a universal NiceGit.app and a DMG under rust/dist/.
# Optional environment:
#   VERSION               defaults to the nicegit crate version in Cargo metadata
#   MACOS_SIGN_IDENTITY   Developer ID Application identity; ad-hoc signing otherwise
#   NOTARY_KEY_PATH, NOTARY_KEY_ID, NOTARY_ISSUER_ID
#                         all three together notarize and staple the DMG

rust_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_dir="$(cd "$rust_dir/.." && pwd)"
cd "$rust_dir"

if [ -z "${VERSION:-}" ]; then
    VERSION="$(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "nicegit"))
')"
fi

app_name="NiceGit"
dist="$rust_dir/dist"
app="$dist/$app_name.app"
staging="$dist/dmg-staging"
dmg="$dist/$app_name-$VERSION-macos-universal.dmg"

if [ -n "${NOTARY_KEY_PATH:-}" ] && [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER_ID:-}" ] \
    && [ -z "${MACOS_SIGN_IDENTITY:-}" ]; then
    echo "Notarization requires MACOS_SIGN_IDENTITY (a Developer ID Application identity)." >&2
    exit 1
fi

cargo build --release --locked --target aarch64-apple-darwin
cargo build --release --locked --target x86_64-apple-darwin

rm -rf "$app" "$staging" "$dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$staging"

lipo -create \
    -output "$app/Contents/MacOS/NiceGit" \
    "target/aarch64-apple-darwin/release/nicegit" \
    "target/x86_64-apple-darwin/release/nicegit"

cp packaging/macos/Info.plist "$app/Contents/Info.plist"
sed -i '' "s/__VERSION__/$VERSION/g" "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
cp assets/NiceGit.icns "$app/Contents/Resources/NiceGit.icns"
cp "$repo_dir/LICENSE" "$app/Contents/Resources/LICENSE"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp \
        --entitlements packaging/macos/entitlements.plist \
        --sign "$MACOS_SIGN_IDENTITY" "$app"
else
    codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"

cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "$app_name" -srcfolder "$staging" -ov -format UDZO "$dmg"
rm -rf "$staging"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$dmg"
fi

if [ -n "${NOTARY_KEY_PATH:-}" ] && [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER_ID:-}" ]; then
    xcrun notarytool submit "$dmg" \
        --key "$NOTARY_KEY_PATH" \
        --key-id "$NOTARY_KEY_ID" \
        --issuer "$NOTARY_ISSUER_ID" \
        --wait
    xcrun stapler staple "$dmg"
fi

echo "$dmg"
