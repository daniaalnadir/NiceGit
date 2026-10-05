import Foundation

/// A deleted local branch with what is needed to recreate it exactly.
public struct GitBranchDeletion: Equatable, Sendable {
    public let name: String
    public let tip: String
    /// The branch's upstream settings, which Git removes along with the branch.
    let upstreamRemote: String?
    let upstreamMerge: String?

    init(name: String, tip: String, upstreamRemote: String?, upstreamMerge: String?) {
        self.name = name; self.tip = tip; self.upstreamRemote = upstreamRemote; self.upstreamMerge = upstreamMerge
    }
}

extension GitClient {
    /// Moves the current branch to `target` as `git reset --keep` does: working files follow,
    /// unrelated local edits are kept, and Git refuses if an edit would be overwritten.
    public func moveBranchKeepingChanges(to target: String, expectedHead: String, expectedBranch: String, in url: URL) throws {
        let snapshot = try loadSnapshot(at: url)
        guard snapshot.operation == nil, snapshot.headHash == expectedHead, snapshot.currentBranch == expectedBranch else {
            throw GitClientError.commandFailed(command: "undo", message: "The checkout changed or a Git operation is in progress, so this can no longer be undone safely.")
        }
        let hash = try run(["rev-parse", "--verify", "--end-of-options", target + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        try run(["reset", "--keep", hash, "--"], in: url)
    }

    /// Deletes a local branch like `deleteBranch`, first saving its tip and upstream settings.
    public func deleteBranchKeepingUndo(_ branch: String, expectedTip: String, in url: URL) throws -> GitBranchDeletion {
        let config = { (key: String) -> String? in
            (try? self.run(["config", "--get", "branch." + branch + "." + key], in: url)).map { $0.hasSuffix("\n") ? String($0.dropLast()) : $0 }
        }
        let deletion = GitBranchDeletion(name: branch, tip: expectedTip, upstreamRemote: config("remote"), upstreamMerge: config("merge"))
        try deleteBranch(branch, expectedTip: expectedTip, in: url)
        return deletion
    }

    /// Recreates a deleted branch at its old tip. Refuses if a branch of that name exists again.
    public func restoreBranch(_ deletion: GitBranchDeletion, in url: URL) throws {
        try run(["check-ref-format", "--branch", deletion.name], in: url)
        let missing = String(repeating: "0", count: deletion.tip.count)
        do {
            // The all-zero old value makes the update fail if the branch already exists.
            try run(["update-ref", "--create-reflog", "-m", "branch: restored by NiceGit", "refs/heads/" + deletion.name, deletion.tip, missing], in: url)
        } catch {
            throw GitClientError.commandFailed(command: "restore branch", message: "A branch named \(deletion.name) exists again, so the deleted one was not restored.")
        }
        if let remote = deletion.upstreamRemote { try run(["config", "branch." + deletion.name + ".remote", remote], in: url) }
        if let merge = deletion.upstreamMerge { try run(["config", "branch." + deletion.name + ".merge", merge], in: url) }
    }

    /// Deletes a branch again after it was restored, only while it still points to the same commit.
    public func deleteRestoredBranch(_ deletion: GitBranchDeletion, in url: URL) throws {
        try run(["update-ref", "-d", "refs/heads/" + deletion.name, deletion.tip], in: url)
        _ = try? run(["config", "--remove-section", "branch." + deletion.name], in: url)
    }
}
