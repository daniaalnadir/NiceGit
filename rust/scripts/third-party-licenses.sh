#!/bin/bash
set -euo pipefail

# Writes THIRD_PARTY_LICENSES.txt, the license texts and copyright notices of the crates built into
# NiceGit, from the dependency tree (see rust/about.toml and rust/about.hbs). It needs cargo-about:
#   cargo install cargo-about --locked --version 0.9.2 --features cli
# Optional argument: where to write the file. A relative path is relative to the current directory.
# Defaults to rust/dist/THIRD_PARTY_LICENSES.txt.

rust_dir="$(cd "$(dirname "$0")/.." && pwd)"

output="${1:-$rust_dir/dist/THIRD_PARTY_LICENSES.txt}"
case "$output" in
    /*) ;;
    *) output="$(pwd)/$output" ;;
esac

if ! command -v cargo-about >/dev/null 2>&1; then
    echo "cargo-about is not installed: cargo install cargo-about --locked --version 0.9.2 --features cli" >&2
    exit 1
fi

mkdir -p "$(dirname "$output")"
cd "$rust_dir"
# --fail makes a crate whose licence cannot be read or accepted an error.
cargo about generate --fail about.hbs -o "$output"

# The interface font and icons are built in too but are not crates, so their notices follow.
{
    printf '\n\nInter font\n==========\n\n'
    printf 'The interface text uses the Inter typeface by The Inter Project Authors\n'
    printf '(https://rsms.me/inter/), subset as described in rust/assets/fonts/README.md.\n\n'
    cat assets/fonts/Inter-LICENSE.txt
    printf '\n\nPhosphor icons\n==============\n\n'
    printf 'The icons are Phosphor Icons (https://phosphoricons.com), copyright Phosphor Icons, used\n'
    printf 'under the MIT License through the egui-phosphor crate, whose license is listed above.\n'
} >>"$output"
echo "$output"
