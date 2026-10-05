import Foundation

/// A `git bisect` session: which commits are known good and bad, which one is checked out for
/// testing, and how many steps remain.
public struct GitBisectStatus: Equatable, Sendable {
    /// The branch or commit Git returns to when the bisect ends.
    public let originalCheckout: String
    public let bad: String?
    public let good: [String]
    public let skipped: [String]
    /// The commit checked out for testing.
    public let testing: String?
    /// Roughly how many more marks are needed, once both a good and a bad commit are known.
    public let remainingSteps: Int?
    /// The first bad commit, once Git has narrowed it to one.
    public let firstBad: String?
}

public enum GitBisectMark: String, Sendable {
    case good, bad, skip
}

extension GitClient {
    public func bisectStatus(in url: URL) throws -> GitBisectStatus? {
        let directory = try gitDirectory(in: url)
        guard FileManager.default.fileExists(atPath: directory.appendingPathComponent("BISECT_START").path) else { return nil }
        return try bisectStatus(gitDirectory: directory, in: url)
    }

    func bisectStatus(gitDirectory: URL, in url: URL) throws -> GitBisectStatus {
        var original = (try? String(contentsOf: gitDirectory.appendingPathComponent("BISECT_START"), encoding: .utf8)) ?? ""
        if original.hasSuffix("\n") { original.removeLast() }
        let refs = try run(["for-each-ref", "--format=%(refname)%00%(objectname)", "refs/bisect/"], in: url)
            .split(separator: "\n").map { $0.split(separator: "\0", maxSplits: 1).map(String.init) }.filter { $0.count == 2 }
        let bad = refs.first { $0[0] == "refs/bisect/bad" }?[1]
        let good = refs.filter { $0[0].hasPrefix("refs/bisect/good-") }.map { $0[1] }
        let skipped = refs.filter { $0[0].hasPrefix("refs/bisect/skip-") }.map { $0[1] }
        let testing = (try? run(["rev-parse", "--verify", "HEAD"], in: url))?.trimmingCharacters(in: .whitespacesAndNewlines)
        var steps: Int?
        var firstBad: String?
        if let bad, !good.isEmpty {
            // Git reports the remaining candidates; one left means the bad commit is the first.
            var values: [String: String] = [:]
            for line in try run(["rev-list", "--bisect-vars", bad, "--not"] + good + ["--"], in: url).split(separator: "\n") {
                let pair = line.split(separator: "=", maxSplits: 1)
                if pair.count == 2 { values[String(pair[0])] = pair[1].trimmingCharacters(in: CharacterSet(charactersIn: "'")) }
            }
            steps = values["bisect_steps"].flatMap(Int.init)
            if values["bisect_all"] == "1" { firstBad = bad; steps = 0 }
        }
        return GitBisectStatus(originalCheckout: original, bad: bad, good: good, skipped: skipped, testing: testing,
                               remainingSteps: steps, firstBad: firstBad)
    }

    /// Starts bisecting with a known bad and good commit and checks out the first one to test.
    public func startBisect(bad: String, good: String, expectedBranch: String, expectedHead: String?, in url: URL) throws {
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: "bisect", in: url)
        guard try currentOperation(in: url) == nil, try bisectStatus(in: url) == nil else {
            throw GitClientError.commandFailed(command: "bisect", message: "Finish the current Git operation or bisect first.")
        }
        guard try loadStatus(in: url).allSatisfy({ $0.kind == .untracked }) else {
            throw GitClientError.commandFailed(command: "bisect", message: "Commit or stash your changes first; bisect checks out other commits.")
        }
        let resolve = { (revision: String) in
            try self.run(["rev-parse", "--verify", "--end-of-options", revision + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let badID = try resolve(bad), goodID = try resolve(good)
        guard badID != goodID, (try? run(["merge-base", "--is-ancestor", goodID, badID], in: url)) != nil else {
            throw GitClientError.commandFailed(command: "bisect", message: "The good commit must be an older ancestor of the bad one.")
        }
        try run(["bisect", "start", badID, goodID, "--"], in: url)
    }

    /// Marks a commit, the tested one by default, and checks out the next to test.
    public func markBisect(_ mark: GitBisectMark, commit: String? = nil, in url: URL) throws {
        guard try bisectStatus(in: url) != nil else {
            throw GitClientError.commandFailed(command: "bisect", message: "No bisect is in progress.")
        }
        let target = try commit.map {
            try run(["rev-parse", "--verify", "--end-of-options", $0 + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        try run(["bisect", mark.rawValue] + (target.map { [$0] } ?? []), in: url)
    }

    /// Ends the bisect and returns to the checkout it started from.
    public func endBisect(in url: URL) throws {
        guard try bisectStatus(in: url) != nil else { return }
        try run(["bisect", "reset"], in: url)
    }
}
