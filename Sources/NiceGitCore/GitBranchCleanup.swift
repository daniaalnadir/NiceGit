import Foundation

/// A local branch that may no longer be needed.
public struct GitCleanupCandidate: Identifiable, Equatable, Sendable {
    public let name: String
    public let tip: String
    public let lastCommitDate: Date?
    public let subject: String
    /// Every commit on the branch is already in the current branch, so deleting it loses nothing.
    public let isMerged: Bool
    public var id: String { name }

    public init(name: String, tip: String, lastCommitDate: Date?, subject: String, isMerged: Bool) {
        self.name = name; self.tip = tip; self.lastCommitDate = lastCommitDate; self.subject = subject; self.isMerged = isMerged
    }
}

extension GitClient {
    /// Local branches merged into the current branch, or with no commits for `inactiveDays`.
    /// The current branch and branches checked out in other worktrees are never listed.
    public func branchCleanupCandidates(inactiveDays: Int = 90, in url: URL) throws -> [GitCleanupCandidate] {
        let merged = Set(try run(["for-each-ref", "--merged", "HEAD", "--format=%(refname)", "refs/heads/"], in: url).split(separator: "\n").map(String.init))
        let checkedOut = Set(GitWorktree.parse(try run(["worktree", "list", "--porcelain", "-z"], in: url)).compactMap(\.branch))
        let cutoff = Date().addingTimeInterval(-Double(max(0, inactiveDays)) * 86_400)
        let output = try run(["for-each-ref", "--format=%(refname)%00%(objectname)%00%(committerdate:unix)%00%(HEAD)%00%(contents:subject)", "refs/heads/"], in: url)
        return output.split(separator: "\n").compactMap { line -> GitCleanupCandidate? in
            let fields = line.split(separator: "\0", maxSplits: 4, omittingEmptySubsequences: false).map(String.init)
            guard fields.count == 5, fields[3] != "*", fields[0].hasPrefix("refs/heads/") else { return nil }
            let name = String(fields[0].dropFirst("refs/heads/".count))
            guard !checkedOut.contains(name), !checkedOut.contains(fields[0]) else { return nil }
            let date = TimeInterval(fields[2]).map { Date(timeIntervalSince1970: $0) }
            let isMerged = merged.contains(fields[0])
            guard isMerged || (date.map { $0 < cutoff } ?? false) else { return nil }
            return GitCleanupCandidate(name: name, tip: fields[1], lastCommitDate: date, subject: fields[4], isMerged: isMerged)
        }.sorted { ($0.lastCommitDate ?? .distantPast) < ($1.lastCommitDate ?? .distantPast) }
    }

    /// Deletes branches that still point to their listed tips, saving each for undo. Unmerged
    /// branches are deleted only with `includeUnmerged`, atomically against their tip.
    public func deleteBranchesKeepingUndo(_ candidates: [GitCleanupCandidate], includeUnmerged: Bool, in url: URL) throws -> [GitBranchDeletion] {
        var deleted: [GitBranchDeletion] = []
        for candidate in candidates {
            if candidate.isMerged {
                deleted.append(try deleteBranchKeepingUndo(candidate.name, expectedTip: candidate.tip, in: url))
            } else if includeUnmerged {
                let config = { (key: String) -> String? in
                    (try? self.run(["config", "--get", "branch." + candidate.name + "." + key], in: url)).map { $0.hasSuffix("\n") ? String($0.dropLast()) : $0 }
                }
                let deletion = GitBranchDeletion(name: candidate.name, tip: candidate.tip, upstreamRemote: config("remote"), upstreamMerge: config("merge"))
                // Git refuses unmerged branches with --delete; remove the ref only if it has not moved.
                try run(["update-ref", "-d", "refs/heads/" + candidate.name, candidate.tip], in: url)
                _ = try? run(["config", "--remove-section", "branch." + candidate.name], in: url)
                deleted.append(deletion)
            }
        }
        return deleted
    }
}
