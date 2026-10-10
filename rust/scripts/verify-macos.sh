#!/bin/bash
set -euo pipefail

# Checks a built NiceGit DMG the way a Mac that downloads it would.
# Usage: verify-macos.sh <dmg>
#
# The app must always have an intact signature. When MACOS_SIGN_IDENTITY is set, it must also be
# signed with a Developer ID Application certificate, with the hardened runtime and a secure
# timestamp, by APPLE_TEAM_ID when that is set. When notarization credentials are set (an App
# Store Connect API key or an Apple ID), Gatekeeper must accept the app as notarized and the DMG
# must carry a stapled ticket. Without those settings the same checks only warn, so a build
# without secrets still completes, clearly marked as unsigned.

dmg="${1:?usage: verify-macos.sh <dmg>}"

strict=false
[ -n "${MACOS_SIGN_IDENTITY:-}" ] && strict=true
notarized=false
if { [ -n "${NOTARY_KEY_PATH:-}" ] && [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER_ID:-}" ]; } \
    || { [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; }; then
    notarized=true
fi

failures=0
passed() { echo "ok: $1"; }
# A failed check is an error for a signed build and a warning for an unsigned one.
problem() {
    if [ "$strict" = true ]; then
        echo "::error::$1"
        failures=$((failures + 1))
    else
        echo "::warning::$1"
    fi
}

mount_dir="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$mount_dir" "$dmg" >/dev/null
trap 'hdiutil detach -quiet "$mount_dir" || true; rmdir "$mount_dir" 2>/dev/null || true' EXIT
app="$mount_dir/NiceGit.app"
if [ ! -d "$app" ]; then
    echo "::error::$dmg does not contain NiceGit.app"
    exit 1
fi

# Every file in the app is signed and unchanged since signing, whoever signed it.
if codesign --verify --strict --deep --verbose=2 "$app"; then
    passed "the app's signature is intact"
else
    echo "::error::The app's signature is broken or incomplete"
    exit 1
fi

details="$(codesign -dvv "$app" 2>&1)"
authority="$(printf '%s\n' "$details" | sed -n 's/^Authority=\(Developer ID Application:.*\)$/\1/p' | head -n1)"
team="$(printf '%s\n' "$details" | sed -n 's/^TeamIdentifier=//p')"
if [ -n "$authority" ]; then
    passed "signed by $authority"
else
    problem "The app is not signed with a Developer ID Application certificate (ad-hoc or other signature)"
fi
if [ -n "${APPLE_TEAM_ID:-}" ]; then
    if [ "$team" = "$APPLE_TEAM_ID" ]; then
        passed "signed by team $team"
    else
        problem "The app is signed by team '$team', not APPLE_TEAM_ID"
    fi
fi
if printf '%s\n' "$details" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]*\([^)]*runtime'; then
    passed "hardened runtime"
else
    problem "The app does not use the hardened runtime, which notarization requires"
fi
if printf '%s\n' "$details" | grep -q '^Timestamp='; then
    passed "secure timestamp"
else
    problem "The app's signature has no secure timestamp, which notarization requires"
fi
if [ "$strict" = true ]; then
    if codesign --verify --strict --verbose=2 "$dmg"; then
        passed "the DMG's signature is intact"
    else
        problem "The DMG is not signed"
    fi
fi

if [ "$notarized" = true ]; then
    # Gatekeeper's own decision, as when someone opens the downloaded app.
    assessment="$(spctl --assess --type execute --verbose=2 "$app" 2>&1 || true)"
    echo "$assessment"
    if printf '%s\n' "$assessment" | grep -q 'accepted' && printf '%s\n' "$assessment" | grep -q 'source=Notarized Developer ID'; then
        passed "Gatekeeper accepts the app as notarized"
    else
        problem "Gatekeeper does not accept the app as notarized"
    fi
    if xcrun stapler validate "$dmg"; then
        passed "the notarization ticket is stapled to the DMG"
    else
        problem "The DMG has no stapled notarization ticket"
    fi
    if spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"; then
        passed "Gatekeeper accepts the DMG"
    else
        problem "Gatekeeper does not accept the DMG"
    fi
elif [ "$strict" = true ]; then
    problem "The app is signed but not notarized: no notarization credentials were given"
fi

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    if [ "$failures" -gt 0 ]; then
        echo "### macOS: $failures signing check(s) failed" >>"$GITHUB_STEP_SUMMARY"
    elif [ "$strict" = true ] && [ "$notarized" = true ]; then
        echo "### macOS: signed with Developer ID and notarized" >>"$GITHUB_STEP_SUMMARY"
    else
        echo "### macOS: unsigned build (ad-hoc signature, not notarized)" >>"$GITHUB_STEP_SUMMARY"
    fi
fi

if [ "$failures" -gt 0 ]; then
    echo "$failures signing check(s) failed." >&2
    exit 1
fi
