#!/bin/bash
set -euo pipefail

# Builds a universal NiceGit.app and a DMG under rust/dist/.
# Optional environment:
#   VERSION               defaults to the nicegit crate version in Cargo metadata
#   MACOS_SIGN_IDENTITY   Developer ID Application identity; ad-hoc signing otherwise
#   Notarization credentials, either set notarizes and staples the DMG:
#     NOTARY_KEY_PATH, NOTARY_KEY_ID, NOTARY_ISSUER_ID   an App Store Connect API key
#     APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID            an Apple ID with an app-specific password

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

# notarytool's sign-in, from whichever complete set of credentials is given.
notary_auth=()
if [ -n "${NOTARY_KEY_PATH:-}" ] || [ -n "${NOTARY_KEY_ID:-}" ] || [ -n "${NOTARY_ISSUER_ID:-}" ]; then
    if [ -z "${NOTARY_KEY_PATH:-}" ] || [ -z "${NOTARY_KEY_ID:-}" ] || [ -z "${NOTARY_ISSUER_ID:-}" ]; then
        echo "Notarizing with an API key needs NOTARY_KEY_PATH, NOTARY_KEY_ID, and NOTARY_ISSUER_ID." >&2
        exit 1
    fi
    notary_auth=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
elif [ -n "${APPLE_ID:-}" ] || [ -n "${APPLE_PASSWORD:-}" ] || [ -n "${APPLE_TEAM_ID:-}" ]; then
    if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_PASSWORD:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ]; then
        echo "Notarizing with an Apple ID needs APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID." >&2
        exit 1
    fi
    notary_auth=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
fi
if [ "${#notary_auth[@]}" -gt 0 ] && [ -z "${MACOS_SIGN_IDENTITY:-}" ]; then
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

if [ "${#notary_auth[@]}" -gt 0 ]; then
    # notarytool can finish without error when Apple rejects the app, so read its verdict.
    result="$(xcrun notarytool submit "$dmg" "${notary_auth[@]}" --wait --timeout 1h --output-format json || true)"
    read -r submission status < <(printf '%s' "$result" | python3 -c '
import json, sys
try:
    r = json.load(sys.stdin)
except ValueError:
    r = {}
print(r.get("id", "-"), r.get("status", "unknown"))
')
    echo "Notarization $submission: $status"
    if [ "$status" != "Accepted" ]; then
        echo "$result" >&2
        if [ "$submission" != "-" ]; then
            xcrun notarytool log "$submission" "${notary_auth[@]}" >&2 || true
        fi
        echo "Apple did not accept the app for notarization." >&2
        exit 1
    fi
    xcrun stapler staple "$dmg"
fi

echo "$dmg"
