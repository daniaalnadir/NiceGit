#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
configuration="${1:-release}"
case "$configuration" in
    debug|release) ;;
    *) echo "Usage: bash scripts/build-app.sh [debug|release]" >&2; exit 1 ;;
esac

swift build --build-system native -c "$configuration"
binary_dir="$(swift build --build-system native -c "$configuration" --show-bin-path)"
app="$PWD/dist/NiceGit.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary_dir/NiceGit" "$app/Contents/MacOS/NiceGit"
cp packaging/Info.plist "$app/Contents/Info.plist"
for bundle in "$binary_dir"/*.bundle; do
    [ -d "$bundle" ] || continue
    ditto "$bundle" "$app/Contents/Resources/$(basename "$bundle")"
done
cp -f .build/checkouts/SwiftTerm/LICENSE "$app/Contents/Resources/SwiftTerm-LICENSE"
cp -f LICENSE "$app/Contents/Resources/NiceGit-LICENSE"
cp -f THIRD_PARTY_NOTICES.md "$app/Contents/Resources/THIRD_PARTY_NOTICES.md"
swift scripts/generate-icon.swift .build/NiceGit.iconset
iconutil -c icns .build/NiceGit.iconset -o "$app/Contents/Resources/NiceGit.icns"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
echo "$app"
