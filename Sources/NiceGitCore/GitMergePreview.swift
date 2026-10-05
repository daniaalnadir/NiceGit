import Foundation

/// What merging a commit into the current checkout would do, worked out without touching
/// the working tree or index.
public struct GitMergePreview: Equatable, Sendable {
    public enum Outcome: Equatable, Sendable {
        /// The current checkout already contains the source.
        case upToDate
        /// The checkout can move forward to the source with no merge commit.
        case fastForward
        /// Git can merge automatically.
        case clean
        /// These paths would need resolving.
        case conflicts([String])
    }
    public let outcome: Outcome
    /// Files the result would change compared with the current checkout.
    public let changedFileCount: Int
    /// Ignored local files sit where incoming files would go; NiceGit refuses such a merge.
    public let blockedByIgnoredFiles: Bool

    public init(outcome: Outcome, changedFileCount: Int, blockedByIgnoredFiles: Bool) {
        self.outcome = outcome; self.changedFileCount = changedFileCount; self.blockedByIgnoredFiles = blockedByIgnoredFiles
    }
}

extension GitClient {
    /// Predicts merging `source` into HEAD with Git's in-memory merge. For a rebase onto
    /// `source` it is an approximation: rebasing replays commits one at a time and can
    /// conflict differently.
    public func previewMerge(of source: String, in url: URL) throws -> GitMergePreview {
        let head = try run(["rev-parse", "--verify", "HEAD"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        let target = try run(["rev-parse", "--verify", "--end-of-options", source + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        let isAncestor = { (older: String, newer: String) in (try? self.run(["merge-base", "--is-ancestor", older, newer], in: url)) != nil }
        let blocked = (try? requireNoIgnoredMergeCollisions(target: target, in: url)) == nil
        if isAncestor(target, head) { return GitMergePreview(outcome: .upToDate, changedFileCount: 0, blockedByIgnoredFiles: false) }
        if isAncestor(head, target) {
            return GitMergePreview(outcome: .fastForward, changedFileCount: try changedFiles(from: head, to: target, in: url), blockedByIgnoredFiles: blocked)
        }
        // With -z and --name-only, Git prints the merged tree, then each conflicted path, then
        // an empty field before its messages. Exit status 1 means conflicts.
        let tokens = try run(["merge-tree", "--write-tree", "--name-only", "-z", head, target], in: url, acceptedStatuses: [0, 1])
            .split(separator: "\0", omittingEmptySubsequences: false)
        guard let tree = tokens.first.map(String.init), !tree.isEmpty else {
            throw GitClientError.commandFailed(command: "merge preview", message: "Git did not report a merge result.")
        }
        var conflicted: [String] = []
        for token in tokens.dropFirst() {
            if token.isEmpty { break }
            if !conflicted.contains(String(token)) { conflicted.append(String(token)) }
        }
        return GitMergePreview(outcome: conflicted.isEmpty ? .clean : .conflicts(conflicted),
                               changedFileCount: try changedFiles(from: head, to: tree, in: url), blockedByIgnoredFiles: blocked)
    }

    private func changedFiles(from older: String, to newer: String, in url: URL) throws -> Int {
        try run(["diff-tree", "-r", "--name-only", "-z", "--no-renames", older, newer, "--"], in: url)
            .split(separator: "\0").count
    }
}
