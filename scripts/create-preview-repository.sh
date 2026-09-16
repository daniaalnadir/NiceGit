#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
preview="$PWD/.build/preview-repository"
if [ -e "$preview" ]; then
    echo "Preview repository already exists: $preview"
    exit 0
fi
mkdir -p "$preview"
git -C "$preview" init --initial-branch=main
git -C "$preview" config user.name "NiceGit Preview"
git -C "$preview" config user.email "preview@example.invalid"
git -C "$preview" commit --allow-empty -m "Start the free Git client"
git -C "$preview" switch -c feature/history
git -C "$preview" commit --allow-empty -m "Add parent-linked commit graph"
git -C "$preview" commit --allow-empty -m "Add searchable history"
git -C "$preview" switch main
git -C "$preview" commit --allow-empty -m "Add repository setup"
git -C "$preview" merge --no-ff --no-edit feature/history
git -C "$preview" switch -c feature/conflicts
git -C "$preview" commit --allow-empty -m "Add three-way conflict comparison"
git -C "$preview" switch main
echo "$preview"
