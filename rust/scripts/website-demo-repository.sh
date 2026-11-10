#!/bin/bash
# Builds a made-up "Weather" repository for the website's screenshots: fictional people at
# example.com, commits from the last few days, a remote, tags, a stash, and uncommitted work.
# (scripts/demo-repository.sh is the smaller one CI's smoke tests use.)
#
# Usage: website-demo-repository.sh <folder>, then render with
#   NICEGIT_SCREENSHOTS=<out> NICEGIT_SCREENSHOT_REPO=<folder>/Weather \
#     cargo test -p nicegit -- --ignored --exact ui_screenshots::screenshots
set -e
root=${1:?Usage: website-demo-repository.sh <folder>}; rm -rf "$root"; mkdir -p "$root"; cd "$root"
git init -q --bare origin.git
git init -q -b main Weather; cd Weather
git config commit.gpgsign false; git config tag.gpgsign false
t=$(( $(date +%s) - 4*86400 ))
c() { # author email message
  git config user.name "$1"; git config user.email "$2"
  t=$((t+5400)); GIT_AUTHOR_DATE="@$t +0000" GIT_COMMITTER_DATE="@$t +0000" git commit -q -m "$3"
}
w() { mkdir -p "$(dirname "$1")"; printf '%s\n' "$2" > "$1"; git add "$1"; }
w README.md "# Weather
A small forecast app."
w App/WeatherApp.swift 'import SwiftUI

@main
struct WeatherApp: App {
    var body: some Scene {
        WindowGroup { TodayView() }
    }
}'
c "Ada Park" ada@example.com "Create the Weather app"
w App/TodayView.swift 'import SwiftUI

struct TodayView: View {
    let forecast: Forecast

    var body: some View {
        VStack(alignment: .leading) {
            Text(forecast.place).font(.title)
            Text("\(forecast.high)° / \(forecast.low)°")
        }
    }
}'
c "Sam Rivera" sam@example.com "Show today's forecast"
w App/Search.swift 'struct Search {
    var query = ""
    func places(matching text: String) -> [String] { [] }
}'
c "Lee Chen" lee@example.com "Add location search"
git tag -a v1.0.0 -m "Weather 1.0"
w Core/Units.swift 'enum Units {
    static func kmh(_ ms: Double) -> Double { ms * 3.6 }
    static func mph(_ ms: Double) -> Double { ms * 2.237 }
}'
c "Sam Rivera" sam@example.com "Convert wind speeds correctly"
w App/TodayView.swift 'import SwiftUI

struct TodayView: View {
    let forecast: Forecast

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(forecast.place).font(.largeTitle.bold())
            Text("\(forecast.high)° / \(forecast.low)°")
                .foregroundStyle(.secondary)
            WindView(speed: forecast.wind)
        }
        .padding()
    }
}'
c "Ada Park" ada@example.com "Polish the today card"
git tag -a v1.1.0 -m "Weather 1.1"
git remote add origin ../origin.git; git push -q origin main; git branch -q -u origin/main
git switch -q -c feature/radar-map
w App/RadarMap.swift 'import MapKit
struct RadarMap { var region: MKCoordinateRegion }'
c "Lee Chen" lee@example.com "Draw the radar map"
w App/RadarMap.swift 'import MapKit
struct RadarMap {
    var region: MKCoordinateRegion
    var frames: [RainFrame] = []
}'
c "Lee Chen" lee@example.com "Animate rain over the last hour"
git push -q -u origin feature/radar-map
git switch -q main
git switch -q -c feature/hourly-forecast
w App/HourlyView.swift 'import SwiftUI

struct HourlyView: View {
    let hours: [Hour]

    var body: some View {
        ScrollView(.horizontal) {
            HStack { ForEach(hours) { HourCell(hour: $0) } }
        }
    }
}'
c "Ada Park" ada@example.com "Add the hourly forecast strip"
w App/HourlyView.swift 'import SwiftUI

struct HourlyView: View {
    let days: [Day]

    var body: some View {
        ScrollView(.horizontal) {
            HStack {
                ForEach(days) { day in
                    Section(day.name) { ForEach(day.hours) { HourCell(hour: $0) } }
                }
            }
        }
    }
}'
c "Sam Rivera" sam@example.com "Group hours by day"
w App/HourCell.swift 'import SwiftUI

struct HourCell: View {
    let hour: Hour
    var body: some View {
        VStack {
            Text(hour.time, format: .dateTime.hour())
            Image(systemName: hour.symbol)
            Text("\(hour.temperature)°")
        }
    }
}'
c "Ada Park" ada@example.com "Highlight the current hour"
git push -q -u origin feature/hourly-forecast
# A stash, then uncommitted work: one staged file, two unstaged.
printf 'Try a compact layout\n' >> README.md; git stash push -q -m "Try a compact layout"
printf 'enum Units {\n    static func kmh(_ ms: Double) -> Double { ms * 3.6 }\n    static func mph(_ ms: Double) -> Double { ms * 2.237 }\n    static func knots(_ ms: Double) -> Double { ms * 1.944 }\n}\n' > Core/Units.swift; git add Core/Units.swift
cat > App/HourlyView.swift <<'S'
import SwiftUI

struct HourlyView: View {
    let days: [Day]
    @State private var selected: Hour.ID?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 12) {
                ForEach(days) { day in
                    Section(day.name) {
                        ForEach(day.hours) { hour in
                            HourCell(hour: hour, isSelected: hour.id == selected)
                                .onTapGesture { selected = hour.id }
                        }
                    }
                }
            }
            .padding(.horizontal)
        }
    }
}
S
printf 'import SwiftUI\n\nstruct SettingsView: View {\n    @AppStorage("units") var units = "metric"\n    var body: some View { Picker("Units", selection: $units) { Text("Metric"); Text("Imperial") } }\n}\n' > App/SettingsView.swift
echo "$PWD"
