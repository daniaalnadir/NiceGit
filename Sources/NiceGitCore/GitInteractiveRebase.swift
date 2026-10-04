import Foundation

public enum GitRebaseAction: Equatable, Sendable {
    case pick
    /// Keep the commit with a new message.
    case reword(String)
    /// Fold into the previous kept commit, adding this commit's message to its message.
    case squash
    /// Fold into the previous kept commit, discarding this commit's message.
    case fixup
    case drop
}

public struct GitRebaseStep: Equatable, Sendable {
    public var commit: GitCommit
    public var action: GitRebaseAction
    public init(commit: GitCommit, action: GitRebaseAction = .pick) { self.commit = commit; self.action = action }
}

/// The commits an interactive rebase from a chosen commit would rewrite.
public struct GitRebasePlan: Sendable {
    /// Oldest first, as Git applies them.
    public let commits: [GitCommit]
    /// The commit the rewritten commits are replayed onto, or nil to rewrite from the root.
    public let base: String?
    /// Commits already on a remote-tracking branch; rewriting them diverges from the remote.
    public let publishedCommits: Set<String>
    /// Full messages by commit, used to combine squashed messages.
    public let messages: [String: String]
}

extension GitClient {
    /// Lists the commits from `oldest` through HEAD on the current branch for editing.
    public func interactiveRebasePlan(from oldest: String, in url: URL) throws -> GitRebasePlan {
        let head = try run(["rev-parse", "--verify", "HEAD"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        let start = try run(["rev-parse", "--verify", "--end-of-options", oldest + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        guard (try? run(["merge-base", "--is-ancestor", start, head], in: url)) != nil else {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "This commit is not part of the current branch's history.")
        }
        let parents = try run(["rev-list", "--parents", "-n", "1", start], in: url).split(separator: " ").dropFirst()
        let base = parents.first.map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
        let range = base.map { $0 + ".." + head } ?? head
        guard try run(["rev-list", "--merges", "--end-of-options", range, "--"], in: url).isEmpty else {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "These commits include a merge. Interactive rebase here keeps history linear, so choose a commit after the latest merge.")
        }
        let format = "%H%x1f%h%x1f%P%x1f%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e"
        let commits = GitLogParser.parse(try run(["log", "--reverse", "--no-color", "--pretty=format:" + format, "--end-of-options", range, "--"], in: url))
        // The range is built from resolved object IDs, so it cannot be read as an option.
        let unpublished = Set(try run(["rev-list", range, "--not", "--remotes", "--"], in: url)
            .split(separator: "\n").map(String.init))
        var messages: [String: String] = [:]
        for commit in commits {
            messages[commit.hash] = try run(["show", "--no-patch", "--format=%B", "--no-color", commit.hash, "--"], in: url)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return GitRebasePlan(commits: commits, base: base,
                             publishedCommits: Set(commits.map(\.hash)).subtracting(unpublished), messages: messages)
    }

    /// Rewrites the current branch from `plan` using `steps`, oldest first. Steps may be
    /// reordered but must cover exactly the planned commits. Messages for rewords and squashes
    /// are stored as Git objects so a rebase that stops for conflicts can still apply them.
    public func interactiveRebase(_ steps: [GitRebaseStep], plan: GitRebasePlan, expectedBranch: String, expectedHead: String, in url: URL) throws {
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: "interactive rebase", in: url)
        guard try currentOperation(in: url) == nil else {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "Finish or abort the current Git operation first.")
        }
        guard try loadStatus(in: url).allSatisfy({ $0.kind == .untracked }) else {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "Commit or stash your changes before rewriting commits.")
        }
        guard steps.map(\.commit.hash).sorted() == plan.commits.map(\.hash).sorted(), plan.commits.last?.hash == expectedHead else {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "The commits to rewrite changed. Refresh and start again.")
        }
        if let first = steps.first(where: { $0.action != .drop }), first.action == .squash || first.action == .fixup {
            throw GitClientError.commandFailed(command: "interactive rebase", message: "The oldest kept commit has nothing to squash into. Pick or reword it instead.")
        }

        func storeMessage(_ message: String) throws -> String {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try Data(message.utf8).write(to: file)
            defer { try? FileManager.default.removeItem(at: file) }
            return try run(["hash-object", "-w", "--", file.path], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        // Each kept commit starts a group; squashes and fixups that follow fold into it.
        var todo: [String] = []
        var groupMessage: String?
        var groupChanged = false
        func finishGroup() throws {
            guard groupChanged, let message = groupMessage else { return }
            let blob = try storeMessage(message)
            todo.append("exec git cat-file blob \(blob) | git commit --amend --only --no-verify --allow-empty --cleanup=whitespace -F -")
        }
        for step in steps {
            let hash = step.commit.hash
            let original = plan.messages[hash] ?? step.commit.subject
            switch step.action {
            case .pick, .reword:
                try finishGroup()
                todo.append("pick " + hash)
                if case let .reword(message) = step.action {
                    guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw GitClientError.emptyCommitMessage }
                    groupMessage = message
                    groupChanged = true
                } else {
                    groupMessage = original
                    groupChanged = false
                }
            case .squash:
                todo.append("fixup " + hash)
                groupMessage = (groupMessage ?? "") + "\n\n" + original
                groupChanged = true
            case .fixup:
                todo.append("fixup " + hash)
            case .drop:
                todo.append("drop " + hash)
            }
        }
        try finishGroup()

        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("NiceGitRebase-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let todoFile = directory.appendingPathComponent("todo")
        try Data((todo.joined(separator: "\n") + "\n").utf8).write(to: todoFile)
        // Git runs the sequence editor through the shell with the todo path as its argument.
        let editor = "cat '" + todoFile.path.replacingOccurrences(of: "'", with: "'\\''") + "' >"
        let arguments = ["-c", "rebase.updateRefs=false", "-c", "rebase.autoStash=false", "-c", "rebase.autoSquash=false",
                         "-c", "rebase.missingCommitsCheck=ignore", "-c", "rebase.abbreviateCommands=false",
                         "rebase", "--interactive", "--empty=drop", "--no-autosquash", "--no-update-refs"]
            + (plan.base.map { ["--end-of-options", $0] } ?? ["--root"])
        do {
            try run(arguments, in: url, environmentOverrides: ["GIT_SEQUENCE_EDITOR": editor])
        } catch {
            if try currentOperation(in: url) == .rebase {
                throw GitClientError.commandFailed(command: "interactive rebase", message: "The rebase stopped, usually for a conflict. Resolve and stage the files, then continue, or abort to return to the original commits.")
            }
            throw error
        }
    }
}
