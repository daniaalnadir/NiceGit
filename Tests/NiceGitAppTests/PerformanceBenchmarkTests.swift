import Foundation
@testable import NiceGit
import NiceGitCore
import Testing

/// Times everyday actions end to end through the app model, including the refresh the user
/// waits for, and counts the Git processes each spawns. Runs only when asked:
///
///     NICEGIT_BENCH_REPO=/path/to/repo NICEGIT_BENCH_OUT=/tmp/results.txt \
///         swift test --filter benchmarkEverydayActions
///
/// The repository is copied for each run, so it is never changed.
@Suite(.serialized)
struct PerformanceBenchmarkTests {
    @Test @MainActor func benchmarkEverydayActions() async throws {
        let env = ProcessInfo.processInfo.environment
        guard let source = env["NICEGIT_BENCH_REPO"] else { return }
        let runs = Int(env["NICEGIT_BENCH_RUNS"] ?? "") ?? 7
        let autoRefresh = UserDefaults.standard.object(forKey: AppModel.autoRefreshKey)
        UserDefaults.standard.set(false, forKey: AppModel.autoRefreshKey)
        defer { UserDefaults.standard.set(autoRefresh, forKey: AppModel.autoRefreshKey) }

        var samples: [String: [(seconds: Double, commands: [GitCommandLog.Entry])]] = [:]
        let order = ["open", "open (fresh index)", "refresh", "quick refresh", "stage file", "unstage file", "stage all", "commit", "undo commit", "redo commit", "discard file"]
        for _ in 0..<runs {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("NiceGitBench-" + UUID().uuidString)
            try copyRepository(from: source, to: root)
            defer { try? FileManager.default.removeItem(at: root) }
            let suite = "NiceGitBench-" + UUID().uuidString
            let defaults = try #require(UserDefaults(suiteName: suite))
            defer { defaults.removePersistentDomain(forName: suite) }
            let model = AppModel(defaults: defaults)

            func measure(_ name: String, on target: AppModel? = nil, _ action: () -> Void) async throws {
                let target = target ?? model
                GitCommandLog.shared.start()
                let clock = ContinuousClock()
                let start = clock.now
                action()
                while target.isLoading { try await Task.sleep(for: .milliseconds(1)) }
                let elapsed = start.duration(to: clock.now)
                let commands = GitCommandLog.shared.stop()
                #expect(target.errorMessage == nil, "\(name): \(target.errorMessage ?? "")")
                samples[name, default: []].append((Double(elapsed.components.attoseconds) / 1e18 + Double(elapsed.components.seconds), commands))
            }
            func edit(_ count: Int) throws {
                for index in 0..<count {
                    let file = root.appendingPathComponent(String(format: "module%03d/file00.swift", index))
                    try ("// edited \(UUID())\n" + String(contentsOf: file, encoding: .utf8)).write(to: file, atomically: true, encoding: .utf8)
                }
            }
            func settle() async throws { while model.isLoading { try await Task.sleep(for: .milliseconds(1)) } }

            try await measure("open") { model.loadRepository(at: root) }
            // Copies have stale index file information; a typical working repository does not.
            let other = AppModel(defaults: defaults)
            try await measure("open (fresh index)", on: other) { other.loadRepository(at: root) }
            try await measure("refresh") { model.refresh() }
            try edit(1)
            try await measure("quick refresh") { model.refreshWorkingTree() }
            let entry = try #require(model.snapshot?.status.first)
            try await measure("stage file") { model.stage(entry) }
            let staged = try #require(model.snapshot?.status.first)
            try await measure("unstage file") { model.unstage(staged) }
            try edit(10)
            model.refreshWorkingTree(); try await settle()
            try await measure("stage all") { model.stageAll() }
            try await measure("commit") { model.commit(message: "Benchmark commit") {} }
            #expect(model.canUndo)
            try await measure("undo commit") { model.moveHistory(redo: false) }
            try await measure("redo commit") { model.moveHistory(redo: true) }
            try edit(1)
            model.refreshWorkingTree(); try await settle()
            let discard = try #require(model.snapshot?.status.first)
            try await measure("discard file") { model.discard(discard) }
        }

        var report = "action              median ms   min ms   git processes   slowest commands (median run)\n"
        for name in order {
            guard let runs = samples[name], !runs.isEmpty else { continue }
            let sorted = runs.sorted { $0.seconds < $1.seconds }
            let median = sorted[sorted.count / 2]
            let slowest = median.commands.sorted { $0.seconds > $1.seconds }.prefix(4)
                .map { String(format: "%@ %.0fms", $0.arguments.prefix(2).joined(separator: " "), $0.seconds * 1000) }.joined(separator: ", ")
            report += String(format: "%-18@ %9.1f %8.1f %15d   %@\n", name as NSString, median.seconds * 1000, sorted[0].seconds * 1000,
                             median.commands.count, slowest as NSString)
        }
        // A timeline of each action's median run shows what still runs one after another.
        for name in ["commit", "undo commit", "discard file", "stage file"] {
            guard let runs = samples[name], !runs.isEmpty else { continue }
            let median = runs.sorted { $0.seconds < $1.seconds }[runs.count / 2]
            report += "\n\(name) timeline (\(String(format: "%.1f", median.seconds * 1000)) ms):\n"
            for entry in median.commands.sorted(by: { $0.start < $1.start }) {
                report += String(format: "  +%5.1f ms  %5.1f ms  %@\n", entry.start * 1000, entry.seconds * 1000, entry.arguments.prefix(3).joined(separator: " ") as NSString)
            }
        }
        print(report)
        if let out = env["NICEGIT_BENCH_OUT"] { try report.write(toFile: out, atomically: true, encoding: .utf8) }
    }

    private func copyRepository(from source: String, to destination: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/cp")
        // Copy-on-write clones on APFS keep each fresh copy fast.
        process.arguments = ["-cR", source, destination.path]
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw CocoaError(.fileWriteUnknown) }
    }
}
