#!/bin/bash
# Creates a small demo repository with branches, tags, a remote, a stash, and uncommitted
# changes, for screenshots and smoke tests. Usage: demo-repository.sh <folder>
set -euo pipefail

target="${1:?Usage: demo-repository.sh <folder>}"
remote="$target.remote.git"
rm -rf "$target" "$remote"
mkdir -p "$target"
cd "$target"

git init -q -b main
git config user.name "Ada Park"
git config user.email "ada@example.invalid"
git config commit.gpgsign false
git config tag.gpgsign false
git config core.autocrlf false

commit() {
    git -c user.name="$1" -c user.email="$(echo "$1" | tr ' ' .)@example.invalid" commit -q -m "$2"
}

mkdir -p App Core
printf '# Weather\n\nA tiny forecast app.\n' > README.md
printf 'struct Forecast {\n    let high: Int\n    let low: Int\n}\n' > Core/Forecast.swift
git add -A && commit "Ada Park" "Create the Weather app"
printf 'func today() -> String {\n    "Sunny"\n}\n' > App/TodayView.swift
git add -A && commit "Sam Rivera" "Show today's forecast"
printf 'func search(_ q: String) {}\n' > App/Search.swift
git add -A && commit "Lee Chen" "Add location search"
git tag v1.0.0
printf 'func convert(_ c: Double) -> Double { c * 9 / 5 + 32 }\n' > Core/Units.swift
git add -A && commit "Sam Rivera" "Convert wind speeds correctly"
printf '\nTemperatures in Celsius and Fahrenheit.\n' >> README.md
git add -A && commit "Ada Park" "Polish the today card"

git switch -q -c feature/radar-map
printf 'func radar() {}\n' > App/Radar.swift
git add -A && commit "Lee Chen" "Draw the radar map"
printf 'func rain() {}\n' >> App/Radar.swift
git add -A && commit "Lee Chen" "Animate rain over the last hour"

git switch -q main
git tag v1.1.0
git switch -q -c feature/hourly-forecast
printf 'func hourly() -> [Int] { [] }\n' > App/HourlyView.swift
git add -A && commit "Ada Park" "Add the hourly forecast strip"
printf '// grouped by day\n' >> App/HourlyView.swift
git add -A && commit "Sam Rivera" "Group hours by day"
printf '// current hour\n' >> App/HourlyView.swift
git add -A && commit "Ada Park" "Highlight the current hour"

git init -q --bare "$remote"
git remote add origin "$remote"
git push -q -u origin main feature/hourly-forecast 2>/dev/null
git push -q origin v1.0.0 2>/dev/null

echo scratch > notes.tmp
git stash push -q -u -m "Try a darker theme" -- notes.tmp
printf 'func settings() {}\n' > App/SettingsView.swift
printf '// metric by default\n' >> Core/Units.swift
git add Core/Units.swift
printf '    // feels like\n' >> App/HourlyView.swift
echo "$target"
