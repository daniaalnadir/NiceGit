import Foundation

public struct GitClient: Sendable {
    private let pathEnvironment = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

    private let control: GitCommandControl?
    private let commandTimeout: TimeInterval

    public init(control: GitCommandControl? = nil, commandTimeout: TimeInterval = 600) {
        self.control = control
        self.commandTimeout = commandTimeout
    }

    public func resolveConflictSide(path: String, incoming: Bool, in url: URL) throws {
        try requireConflict(path: path, in: url)
        try run(["checkout", incoming ? "--theirs" : "--ours", "--", path], in: url)
        try stage(path: path, in: url)
    }

    public func resolveConflictDeletion(path: String, in url: URL) throws {
        try requireConflict(path: path, in: url)
        try run(["rm", "--force", "--", path], in: url)
    }

    private func requireConflict(path: String, in url: URL) throws {
        guard try loadSnapshot(at: url).status.contains(where: { $0.path == path && $0.kind == .conflicted }) else {
            throw GitClientError.commandFailed(command: "resolve conflict", message: "This file no longer has an unresolved conflict.")
        }
    }

    public func createTag(name: String, target: String, message: String? = nil, in url: URL) throws {
        try run(["check-ref-format", "refs/tags/" + name], in: url)
        if let message {
            guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw GitClientError.commandFailed(command: "create annotated tag", message: "Enter a tag message.")
            }
            try run(["-c", "tag.gpgSign=false", "tag", "--annotate", "--message", message, "--", name, target], in: url)
        } else {
            try run(["-c", "tag.gpgSign=false", "tag", "--", name, target], in: url)
        }
    }

    public func deleteTag(name: String, expectedTip: String? = nil, in url: URL) throws {
        if let expectedTip {
            try run(["update-ref", "-d", "refs/tags/" + name, expectedTip], in: url)
        } else {
            try run(["tag", "--delete", "--", name], in: url)
        }
    }

    func conflictVersion(path: String, stage: Int, in url: URL) -> String? {
        guard let value = try? run(["show", ":\(stage):\(path)"], in: url), !value.contains("\0") else { return nil }
        return value
    }

    public func initialize(at url: URL) throws {
        try run(["init", "--initial-branch=main"], in: url)
    }

    public func identity(in url: URL) -> (name: String, email: String) {
        let name = (try? run(["config", "--get", "user.name"], in: url)) ?? ""
        let email = (try? run(["config", "--get", "user.email"], in: url)) ?? ""
        return (name.trimmingCharacters(in: .whitespacesAndNewlines), email.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    public func setIdentity(name: String, email: String, in url: URL) throws {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !email.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw GitClientError.commandFailed(command: "config", message: "Name and email are required.")
        }
        try run(["config", "--local", "user.name", name], in: url)
        try run(["config", "--local", "user.email", email], in: url)
    }

    public func addRemote(name: String, address: String, in url: URL) throws {
        try run(["remote", "add", "--", name, address], in: url)
    }

    public func remoteAddress(name: String, in url: URL) throws -> String {
        var address = try run(["remote", "get-url", "--", name], in: url)
        if address.hasSuffix("\n") { address.removeLast() }
        return address
    }

    public func start(_ operation: GitOperation, target: String, mainline: Int? = nil, expectedHead: String? = nil, expectedBranch: String? = nil, expectedSourceBranch: GitBranch? = nil, in url: URL) throws {
        let snapshot = try loadSnapshot(at: url)
        guard expectedHead == nil || snapshot.headHash == expectedHead,
              expectedBranch == nil || snapshot.currentBranch == expectedBranch else {
            throw GitClientError.commandFailed(command: operation.rawValue, message: "The current branch or HEAD changed since this action was selected. Refresh and review the operation again.")
        }
        guard snapshot.operation == nil, snapshot.status.isEmpty else {
            throw GitClientError.commandFailed(command: operation.rawValue, message: "Commit or stash changes and finish the current operation first.")
        }
        let hash = try run(["rev-parse", "--verify", "--end-of-options", target + "^{commit}"], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let expectedSourceBranch {
            try requireSelectedBranchTip(expectedSourceBranch, command: operation.rawValue, in: url)
            guard hash == expectedSourceBranch.tip else {
                throw GitClientError.commandFailed(command: operation.rawValue, message: "The selected branch changed since this action was selected. Refresh and review it again.")
            }
        }
        var arguments = [operation.rawValue]
        if operation == .merge || operation == .revert { arguments.append("--no-edit") }
        if let mainline, operation == .revert || operation == .cherryPick { arguments += ["--mainline", String(mainline)] }
        try run(arguments + [hash], in: url)
    }

    public func reset(to target: String, mode: GitResetMode, expectedHead: String, expectedBranch: String, in url: URL) throws {
        let snapshot = try loadSnapshot(at: url)
        guard snapshot.operation == nil, snapshot.headHash == expectedHead, snapshot.currentBranch == expectedBranch else {
            throw GitClientError.commandFailed(command: "reset", message: "The checkout changed or a Git operation is in progress. Refresh and review the reset again.")
        }
        let hash = try run(["rev-parse", "--verify", "--end-of-options", target + "^{commit}"], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        try run(["reset", "--" + mode.rawValue, hash, "--"], in: url)
    }

    public func continueOperation(_ operation: GitOperation, in url: URL) throws {
        try run([operation.rawValue, "--continue"], in: url)
    }

    public func abortOperation(_ operation: GitOperation, in url: URL) throws {
        try run([operation.rawValue, "--abort"], in: url)
    }

    private func currentOperation(in url: URL) throws -> GitOperation? {
        let markers: [(String, GitOperation)] = [("rebase-merge", .rebase), ("rebase-apply", .rebase), ("MERGE_HEAD", .merge), ("CHERRY_PICK_HEAD", .cherryPick), ("REVERT_HEAD", .revert)]
        var gitDirectory = try run(["rev-parse", "--absolute-git-dir"], in: url)
        if gitDirectory.hasSuffix("\n") { gitDirectory.removeLast() }
        let directory = URL(fileURLWithPath: gitDirectory, isDirectory: true)
        for (marker, operation) in markers {
            if FileManager.default.fileExists(atPath: directory.appendingPathComponent(marker).path) { return operation }
        }
        return nil
    }

    public func saveStash(message: String, includeUntracked: Bool, in url: URL) throws {
        let previous = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        try pushStash(message: message, includeUntracked: includeUntracked, in: url)
        guard let saved = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines), saved != previous else {
            throw GitClientError.commandFailed(command: "stash", message: "Git did not save any changes. Check the untracked-file setting and any dirty submodules.")
        }
        let remaining = try loadStatus(in: url)
        if remaining.contains(where: { includeUntracked || $0.kind != .untracked }) {
            throw GitClientError.commandFailed(command: "stash", message: "Some changes were saved in stash \(saved.prefix(12)), but changes remain in the working tree. Check submodules before proceeding.")
        }
    }

    private func pushStash(message: String, includeUntracked: Bool, in url: URL) throws {
        let stashMessage = message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "WIP on \(currentBranch(in: url))" : message
        try run(["stash", "push"] + (includeUntracked ? ["--include-untracked"] : []) + ["-m", stashMessage], in: url)
    }

    public func applyStash(_ stash: GitStash, in url: URL) throws {
        guard try listStashes(in: url).contains(where: { $0.hash == stash.hash }) else {
            throw GitClientError.commandFailed(command: "stash apply", message: "This stash no longer exists. Refresh the repository.")
        }
        try run(["stash", "apply", "--index", stash.hash], in: url)
    }

    public func popStash(_ stash: GitStash, in url: URL) throws {
        guard try listStashes(in: url).contains(where: { $0.hash == stash.hash }) else {
            throw GitClientError.commandFailed(command: "stash pop", message: "This stash no longer exists. Refresh the repository.")
        }
        try applyStash(stash, in: url)
        do {
            try dropStash(stash, in: url)
        } catch {
            throw GitClientError.commandFailed(command: "stash pop", message: "The stash was applied, but could not be removed. Do not apply it again. Refresh and inspect the stash list before deleting it. \(error.localizedDescription)")
        }
    }

    public func dropStash(_ stash: GitStash, in url: URL) throws {
        // Resolve the current reference because stash positions can change outside the app.
        guard let current = try listStashes(in: url).first(where: { $0.hash == stash.hash }) else {
            throw GitClientError.commandFailed(command: "stash drop", message: "This stash no longer exists. Refresh the repository.")
        }
        try run(["stash", "drop", current.reference], in: url)
    }

    public func listStashes(in url: URL) throws -> [GitStash] {
        try run(["stash", "list", "--format=%H%x09%gd%x09%s"], in: url)
            .split(separator: "\n").compactMap { line in
                let fields = line.split(separator: "\t", maxSplits: 2, omittingEmptySubsequences: false).map(String.init)
                guard fields.count == 3 else { return nil }
                return GitStash(hash: fields[0], reference: fields[1], message: fields[2])
            }
    }

    public func applyPatch(_ contents: Data, in url: URL) throws {
        guard try currentOperation(in: url) == nil else {
            throw GitClientError.commandFailed(command: "apply", message: "Finish or abort the current Git operation before applying a patch.")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let patch = directory.appendingPathComponent("import.patch")
        try contents.write(to: patch)
        // Both commands read the same captured bytes, not a changing external file.
        try run(["apply", "--check", "--", patch.path], in: url)
        try run(["apply", "--", patch.path], in: url)
    }

    public func exportCommitPatch(hash: String, in url: URL) throws -> String {
        let resolved = try run(["rev-parse", "--verify", "--end-of-options", hash + "^{commit}"], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let parents = try run(["rev-list", "--parents", "-n", "1", resolved], in: url).split(whereSeparator: \.isWhitespace)
        guard parents.count <= 2 else {
            throw GitClientError.commandFailed(command: "format-patch", message: "Merge commits cannot be exported as a single email patch. Select an individual commit.")
        }
        let files = try run(["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", resolved, "--"], in: url)
        guard !files.isEmpty else {
            throw GitClientError.commandFailed(command: "format-patch", message: "This commit has no file changes to export.")
        }
        let patch = try run(["format-patch", "--stdout", "--root", "--no-cover-letter", "--no-signature", "--no-thread", "--no-attach", "--binary", "--full-index", "--no-ext-diff", "--no-textconv", "-1", resolved, "--"], in: url)
        guard !patch.isEmpty else {
            throw GitClientError.commandFailed(command: "format-patch", message: "This commit has no exportable patch.")
        }
        return patch
    }

    public func clone(source: String, to destination: URL) throws {
        try run(["clone", "--", source, destination.path], in: destination.deletingLastPathComponent())
    }

    public func diff(path: String, staged: Bool, untracked: Bool = false, originalPath: String? = nil, in repositoryURL: URL) throws -> String {
        if untracked {
            return try run(["diff", "--no-index", "--no-ext-diff", "--no-color", "--", "/dev/null", path], in: repositoryURL, acceptedStatuses: [0, 1])
        }
        return try run(["diff", "--no-ext-diff", "--no-color"] + (staged ? ["--cached"] : []) + ["--", path] + (originalPath.map { [$0] } ?? []), in: repositoryURL)
    }

    public func commitDiff(hash: String, path: String? = nil, in repositoryURL: URL) throws -> String {
        try run(["show", "--first-parent", "-m", "--format=fuller", "--stat", "--patch", "--no-ext-diff", "--no-color", hash, "--"] + (path.map { [$0] } ?? []), in: repositoryURL)
    }

    public func commitFileDiff(hash: String, path: String, in repositoryURL: URL) throws -> String {
        try run(["show", "--first-parent", "-m", "--format=", "--patch", "--unified=3", "--no-renames", "--no-ext-diff", "--no-color", hash, "--", path], in: repositoryURL)
    }

    public func commitMessage(hash: String, in repositoryURL: URL) throws -> String {
        try run(["show", "--no-patch", "--format=%B", "--no-color", hash, "--"], in: repositoryURL)
    }

    public func commitFiles(hash: String, in repositoryURL: URL) throws -> [String] {
        try run(["show", "--format=", "--name-only", "--first-parent", "-m", "-z", hash, "--"], in: repositoryURL)
            .split(separator: "\0").map(String.init)
    }

    public func commitFileChanges(hash: String, in repositoryURL: URL) throws -> [GitCommitFileChange] {
        let output = try run(["diff-tree", "--root", "--no-commit-id", "--first-parent", "-m", "-r", "--no-renames", "--name-status", "-z", hash, "--"], in: repositoryURL)
        let fields = output.split(separator: "\0", omittingEmptySubsequences: false)
        return stride(from: 0, to: max(0, fields.count - 1), by: 2).map {
            GitCommitFileChange(path: String(fields[$0 + 1]), status: String(fields[$0]))
        }.sorted { $0.path < $1.path }
    }

    public func stashDiff(hash: String, in repositoryURL: URL) throws -> String {
        try run(["stash", "show", "--include-untracked", "--patch", "--stat", "--no-ext-diff", "--no-color", hash], in: repositoryURL)
    }

    public func stashFiles(hash: String, in repositoryURL: URL) throws -> [String] {
        try run(["stash", "show", "--include-untracked", "--name-only", "-z", hash], in: repositoryURL)
            .split(separator: "\0").map(String.init).sorted()
    }

    public func loadSnapshot(at selectedURL: URL, historyLimit: Int = 200) throws -> RepositorySnapshot {
        let rootPath = try repositoryRoot(for: selectedURL)
        let rootURL = URL(fileURLWithPath: rootPath)
        let status = try GitStatusParser.parseNullTerminated(run(["status", "--porcelain=v1", "-z", "--untracked-files=all"], in: rootURL))
        let branches = try GitBranchParser.parse(run(["branch", "--all", "--format=%(refname)%09%(HEAD)%09%(objectname)%09%(contents:subject)%09%(upstream)"], in: rootURL))
        let branch = branches.first(where: { $0.isCurrent && !$0.name.hasPrefix("(") })?.name ?? currentBranch(in: rootURL)
        let headHash = (try? run(["rev-parse", "--verify", "HEAD"], in: rootURL))?.trimmingCharacters(in: .whitespacesAndNewlines)
        let head = headHash != nil ? ["HEAD"] : []
        let commits = GitLogParser.parse(try run([
            "log",
            "--exclude=refs/stash",
            "--all",
            "--topo-order",
            "--decorate=short",
            "--date=relative",
            "-n",
            String(max(1, historyLimit) + 1),
            "--pretty=format:%H%x1f%h%x1f%P%x1f%D%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e"
        ] + head + ["--"], in: rootURL))
        let remoteOutput = try run(["remote", "-v"], in: rootURL)
        let remotes = GitRemoteParser.parse(remoteOutput)

        var snapshot = RepositorySnapshot(
            rootPath: rootPath,
            name: rootURL.lastPathComponent,
            currentBranch: branch,
            status: status,
            branches: branches,
            commits: Array(commits.prefix(max(1, historyLimit))),
            remotes: remotes
        )
        snapshot.remoteAddresses = GitRemoteParser.addresses(remoteOutput)
        snapshot.remoteFetchAddresses = GitRemoteParser.allAddresses(remoteOutput, direction: "fetch")
        snapshot.remotePushAddresses = GitRemoteParser.allAddresses(remoteOutput, direction: "push")
        snapshot.stashes = try listStashes(in: rootURL)
        snapshot.headHash = headHash
        snapshot.worktrees = GitWorktree.parse(try run(["worktree", "list", "--porcelain", "-z"], in: rootURL))
        let tagLines = try run(["for-each-ref", "--sort=-version:refname", "--format=%(refname:strip=2)%09%(objectname)", "refs/tags"], in: rootURL)
        for line in tagLines.split(separator: "\n") {
            let parts = line.split(separator: "\t", maxSplits: 1)
            guard parts.count == 2 else { continue }
            let name = String(parts[0])
            snapshot.tags.append(name)
            snapshot.tagTips[name] = String(parts[1])
        }
        if let upstream = try? run(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], in: rootURL),
           let counts = try? run(["rev-list", "--left-right", "--count", "HEAD...@{upstream}", "--"], in: rootURL) {
            let values = counts.split(whereSeparator: { $0.isWhitespace }).compactMap { Int($0) }
            snapshot.upstream = upstream.trimmingCharacters(in: .whitespacesAndNewlines)
            if values.count == 2 {
                snapshot.ahead = values[0]
                snapshot.behind = values[1]
            }
        }
        snapshot.hasMoreCommits = commits.count > max(1, historyLimit)
        snapshot.operation = try currentOperation(in: rootURL)
        return snapshot
    }

    public func loadStatus(in repositoryURL: URL) throws -> [GitStatusEntry] {
        try GitStatusParser.parseNullTerminated(run(["status", "--porcelain=v1", "-z", "--untracked-files=all"], in: repositoryURL))
    }

    public func stage(path: String, in repositoryURL: URL) throws {
        try run(["add", "--", path], in: repositoryURL)
    }

    public func stageAll(in repositoryURL: URL) throws {
        try run(["add", "--all"], in: repositoryURL)
    }

    public func unstage(path: String, originalPath: String? = nil, in repositoryURL: URL) throws {
        if (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) == nil {
            try run(["rm", "--cached", "--", path], in: repositoryURL)
        } else {
            try run(["restore", "--staged", "--", path] + (originalPath.map { [$0] } ?? []), in: repositoryURL)
        }
    }

    public func unstageAll(in repositoryURL: URL) throws {
        if (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) == nil {
            try run(["rm", "--cached", "-r", "--", "."], in: repositoryURL)
        } else {
            try run(["restore", "--staged", "."], in: repositoryURL)
        }
    }

    public func discard(_ entry: GitStatusEntry, in repositoryURL: URL) throws {
        guard try loadStatus(in: repositoryURL).contains(entry) else {
            throw GitClientError.commandFailed(command: "discard", message: "This file changed since it was selected. Refresh and review it again.")
        }
        if entry.kind == .untracked {
            try run(["clean", "--force", "--", entry.path], in: repositoryURL)
            if try loadStatus(in: repositoryURL).contains(where: { $0.path == entry.path }) {
                throw GitClientError.commandFailed(command: "discard", message: "Git could not remove this untracked path. Nested repositories require manual removal.")
            }
        } else if (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) == nil {
            try run(["rm", "--force", "--", entry.path], in: repositoryURL)
        } else {
            try run(["restore", "--source=HEAD", "--staged", "--worktree", "--", entry.path] + (entry.originalPath.map { [$0] } ?? []), in: repositoryURL)
        }
    }

    public func commit(message: String, in repositoryURL: URL) throws {
        let trimmedMessage = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedMessage.isEmpty else {
            throw GitClientError.emptyCommitMessage
        }

        try run(["commit", "-m", trimmedMessage], in: repositoryURL)
    }

    @discardableResult
    public func checkout(branch: String, expectedTip: String? = nil, in repositoryURL: URL) throws -> Bool {
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: repositoryURL) }
        return try switchPreservingChanges(["switch", "--no-overwrite-ignore", "--", branch], to: branch, in: repositoryURL)
    }

    public func amendMessage(_ message: String, expectedHead: String, in url: URL) throws {
        guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw GitClientError.emptyCommitMessage }
        let snapshot = try loadSnapshot(at: url)
        guard snapshot.operation == nil, snapshot.headHash == expectedHead else {
            throw GitClientError.commandFailed(command: "amend message", message: "HEAD changed or a Git operation is in progress. Refresh and select the current HEAD commit.")
        }
        try run(["commit", "--amend", "--only", "--message", message], in: url)
    }

    public func createWorktree(branch: String, expectedTip: String? = nil, at destination: URL, in repositoryURL: URL) throws {
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: repositoryURL) }
        else { try run(["show-ref", "--verify", "--quiet", "refs/heads/" + branch], in: repositoryURL) }
        try run(["worktree", "add", "--", destination.path, branch], in: repositoryURL)
    }

    public func renameBranch(_ branch: String, to name: String, expectedTip: String? = nil, in url: URL) throws {
        try run(["check-ref-format", "--branch", name], in: url)
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: url) }
        try run(["branch", "--move", "--", branch, name], in: url)
    }

    public func deleteBranch(_ branch: String, expectedTip: String? = nil, in url: URL) throws {
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: url) }
        try run(["branch", "--delete", "--", branch], in: url)
    }

    private func requireBranchTip(_ branch: String, expectedTip: String, in url: URL) throws {
        let current = try run(["rev-parse", "--verify", "--end-of-options", "refs/heads/" + branch], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard current == expectedTip else {
            throw GitClientError.commandFailed(command: "branch", message: "This branch changed since it was selected. Refresh and review it again.")
        }
    }

    private func requireSelectedBranchTip(_ branch: GitBranch, command: String, in url: URL) throws {
        let reference = branch.isRemote
            ? "refs/" + branch.name
            : "refs/heads/" + branch.name
        let current = try run(["rev-parse", "--verify", "--end-of-options", reference], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard current == branch.tip else {
            throw GitClientError.commandFailed(command: command, message: "The selected branch changed since this action was selected. Refresh and review it again.")
        }
    }

    @discardableResult
    public func checkoutRemote(branch: String, expectedTip: String? = nil, in url: URL) throws -> Bool {
        let reference: String
        if branch.hasPrefix("refs/remotes/") { reference = branch }
        else if branch.hasPrefix("remotes/") { reference = "refs/" + branch }
        else { reference = "refs/remotes/" + branch }
        if let expectedTip {
            let current = try run(["rev-parse", "--verify", "--end-of-options", reference], in: url)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard current == expectedTip else {
                throw GitClientError.commandFailed(command: "checkout remote branch", message: "This remote branch changed since it was selected. Refresh and review it again.")
            }
        } else {
            try run(["show-ref", "--verify", "--quiet", reference], in: url)
        }
        let branches = try GitBranchParser.parse(run(["branch", "--all", "--format=%(refname)%09%(HEAD)%09%(objectname)%09%(contents:subject)%09%(upstream)"], in: url))
        let tracking = branches.filter { !$0.isRemote && $0.upstream == reference }
        if tracking.count > 1 {
            throw GitClientError.commandFailed(command: "checkout remote branch", message: "Several local branches track this remote branch. Choose the desired branch in Local.")
        }
        if let existing = tracking.first {
            return try checkout(branch: existing.name, expectedTip: existing.tip, in: url)
        } else {
            return try switchPreservingChanges(["switch", "--no-overwrite-ignore", "--track", "--", reference], to: reference, in: url)
        }
    }

    private func switchPreservingChanges(_ arguments: [String], to branch: String, in url: URL) throws -> Bool {
        guard try !loadStatus(in: url).isEmpty else {
            try run(arguments, in: url)
            return false
        }
        let source = currentBranch(in: url)
        if branch == source { return false }
        let previousStash = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        try pushStash(message: "NiceGit: changes from \(source) before switching to \(branch)", includeUntracked: true, in: url)
        guard let stashHash = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines), stashHash != previousStash else {
            throw GitClientError.commandFailed(command: "switch branch", message: "Git could not save all working changes. The branch was not switched. Check the working tree and Stashes before retrying.")
        }
        do {
            guard try loadStatus(in: url).isEmpty else {
                throw GitClientError.commandFailed(command: "switch branch", message: "Git could not stash every change, including changes inside submodules. The branch was not switched.")
            }
            try run(arguments, in: url)
            return true
        } catch {
            do {
                try restoreSavedStash(stashHash, in: url)
            } catch let restoreError {
                throw GitClientError.commandFailed(command: "switch branch", message: "Switch failed: \(error.localizedDescription)\nYour changes are saved in stash \(stashHash.prefix(12)). Automatic restoration also failed: \(restoreError.localizedDescription)")
            }
            throw error
        }
    }

    private func restoreSavedStash(_ hash: String, in url: URL) throws {
        try run(["stash", "apply", "--index", hash], in: url)
        if let saved = try listStashes(in: url).first(where: { $0.hash == hash }) {
            do {
                try run(["stash", "drop", saved.reference], in: url)
            } catch {
                throw GitClientError.commandFailed(command: "switch branch", message: "Changes were restored, but stash \(hash.prefix(12)) could not be removed. Do not apply it again. \(error.localizedDescription)")
            }
        }
    }

    public func publish(remote: String, expectedBranch: String? = nil, expectedHead: String? = nil, expectedPushAddresses: [String: [String]]? = nil, in url: URL) throws {
        let branch = try run(["symbolic-ref", "--quiet", "--short", "HEAD"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        let head = try run(["rev-parse", "--verify", "HEAD"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        guard (expectedBranch == nil || expectedBranch == branch),
              (expectedHead == nil || expectedHead == head) else {
            throw GitClientError.commandFailed(command: "publish", message: "The current branch changed since it was selected. Refresh and review the publish again.")
        }
        if let expectedPushAddresses {
            try requireRemoteAddresses(expectedPushAddresses, remote: remote, push: true, command: "publish", in: url)
        }
        try run(["remote", "get-url", "--push", "--", remote], in: url)
        let reference = "refs/heads/" + branch
        try run(["-c", "remote." + remote + ".mirror=false", "push", "--set-upstream", "--no-follow-tags", "--recurse-submodules=no", "--", remote, reference + ":" + reference], in: url)
    }

    public func createBranch(named name: String, expectedBranch: String? = nil, expectedHead: String? = nil, in repositoryURL: URL) throws {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty else {
            throw GitClientError.emptyBranchName
        }
        if let expectedBranch {
            let head = (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard currentBranch(in: repositoryURL) == expectedBranch, head == expectedHead else {
                throw GitClientError.commandFailed(command: "create branch", message: "The current checkout changed since Create branch was selected. Refresh and review it again.")
            }
        }
        try run(["checkout", "-b", trimmedName], in: repositoryURL)
    }

    public func pushBranch(_ branch: String, to remote: String, expectedTip: String? = nil, expectedPushAddresses: [String: [String]]? = nil, in url: URL) throws {
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: url) }
        else { try run(["show-ref", "--verify", "--quiet", "refs/heads/" + branch], in: url) }
        if let expectedPushAddresses {
            try requireRemoteAddresses(expectedPushAddresses, remote: remote, push: true, command: "push", in: url)
        }
        try run(["remote", "get-url", "--push", "--", remote], in: url)
        let reference = "refs/heads/" + branch
        try run(["-c", "remote." + remote + ".mirror=false", "push", "--no-follow-tags", "--recurse-submodules=no", "--", remote, reference + ":" + reference], in: url)
    }

    public func createBranch(named name: String, startingAt target: String, expectedSourceBranch: GitBranch? = nil, in url: URL) throws {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { throw GitClientError.emptyBranchName }
        try run(["check-ref-format", "--branch", name], in: url)
        let commit = try run(["rev-parse", "--verify", "--end-of-options", target + "^{commit}"], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let expectedSourceBranch {
            try requireSelectedBranchTip(expectedSourceBranch, command: "branch", in: url)
            guard commit == expectedSourceBranch.tip else {
                throw GitClientError.commandFailed(command: "branch", message: "The selected branch changed since this action was selected. Refresh and review it again.")
            }
        }
        try run(["branch", "--no-track", "--", name, commit], in: url)
    }

    public func fetch(in repositoryURL: URL) throws {
        try run(["fetch", "--all", "--prune"], in: repositoryURL)
    }

    public func setUpstream(branch: String, remoteBranch: String?, expectedTip: String? = nil, in url: URL) throws {
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: url) }
        else { try run(["show-ref", "--verify", "--quiet", "refs/heads/" + branch], in: url) }
        if let remoteBranch {
            let reference = "refs/remotes/" + remoteBranch
            try run(["show-ref", "--verify", "--quiet", reference], in: url)
            try run(["branch", "--set-upstream-to=" + reference, "--", branch], in: url)
        } else {
            try run(["branch", "--unset-upstream", "--", branch], in: url)
        }
    }

    public func pull(expectedBranch: String? = nil, expectedHead: String? = nil, expectedUpstream: String? = nil, expectedFetchAddresses: [String: [String]]? = nil, in repositoryURL: URL) throws {
        if expectedBranch != nil || expectedHead != nil {
            let branch = try run(["symbolic-ref", "--quiet", "--short", "HEAD"], in: repositoryURL)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let head = try run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard (expectedBranch == nil || expectedBranch == branch),
                  (expectedHead == nil || expectedHead == head) else {
                throw GitClientError.commandFailed(command: "pull", message: "The current branch changed since Pull was selected. Refresh and review it again.")
            }
        }
        if let expectedUpstream {
            try requireUpstream(expectedUpstream, command: "pull", in: repositoryURL)
        }
        if let expectedFetchAddresses {
            let branch = try run(["symbolic-ref", "--quiet", "--short", "HEAD"], in: repositoryURL)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let remote = try run(["config", "--get", "branch." + branch + ".remote"], in: repositoryURL)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            try requireRemoteAddresses(expectedFetchAddresses, remote: remote, push: false, command: "pull", in: repositoryURL)
        }
        try run(["pull", "--ff-only"], in: repositoryURL)
    }

    public func push(expectedBranch: String? = nil, expectedHead: String? = nil, expectedUpstream: String? = nil, expectedPushAddresses: [String: [String]]? = nil, in repositoryURL: URL) throws {
        let branch = try run(["symbolic-ref", "--quiet", "--short", "HEAD"], in: repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let head = try run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard (expectedBranch == nil || expectedBranch == branch),
              (expectedHead == nil || expectedHead == head) else {
            throw GitClientError.commandFailed(command: "push", message: "The current branch changed since it was selected. Refresh and review the push again.")
        }
        let remote = try run(["config", "--get", "branch." + branch + ".remote"], in: repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let upstream = try run(["config", "--get", "branch." + branch + ".merge"], in: repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard remote != ".", !remote.isEmpty, upstream.hasPrefix("refs/heads/") else {
            throw GitClientError.commandFailed(command: "push", message: "Set a remote branch as the upstream before pushing.")
        }
        if let expectedUpstream {
            try requireUpstream(expectedUpstream, command: "push", in: repositoryURL)
        }
        if let expectedPushAddresses {
            try requireRemoteAddresses(expectedPushAddresses, remote: remote, push: true, command: "push", in: repositoryURL)
        }
        try run(["remote", "get-url", "--push", "--", remote], in: repositoryURL)
        try run(["-c", "remote." + remote + ".mirror=false", "push", "--no-follow-tags", "--recurse-submodules=no", "--", remote, head + ":" + upstream], in: repositoryURL)
    }

    private func requireUpstream(_ expected: String, command: String, in repositoryURL: URL) throws {
        let current = (try? run(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], in: repositoryURL))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard current == expected else {
            throw GitClientError.commandFailed(command: command, message: "The upstream branch changed since this action was selected. Refresh and review it again.")
        }
    }

    private func requireRemoteAddresses(_ expected: [String: [String]], remote: String, push: Bool, command: String, in repositoryURL: URL) throws {
        let arguments = ["remote", "get-url"] + (push ? ["--push"] : []) + ["--all", "--", remote]
        let current = (try? run(arguments, in: repositoryURL))?
            .split(separator: "\n").map(String.init)
        guard let displayed = expected[remote], !displayed.isEmpty, current == displayed else {
            throw GitClientError.commandFailed(command: command, message: "The remote address changed since this action was selected. Refresh and review it again.")
        }
    }

    private func repositoryRoot(for selectedURL: URL) throws -> String {
        let output = try run(["rev-parse", "--show-toplevel"], in: selectedURL)
        // Git appends one line terminator; any preceding whitespace belongs to the path.
        return output.hasSuffix("\n") ? String(output.dropLast()) : output
    }

    private func currentBranch(in repositoryURL: URL) -> String {
        let branch = ((try? run(["branch", "--show-current"], in: repositoryURL)) ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)

        if !branch.isEmpty {
            return branch
        }

        let detachedHead = ((try? run(["rev-parse", "--short", "HEAD"], in: repositoryURL)) ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return detachedHead.isEmpty ? "No commits yet" : "Detached HEAD \(detachedHead)"
    }

    @discardableResult
    func run(_ arguments: [String], in directory: URL, acceptedStatuses: Set<Int32> = [0]) throws -> String {
        guard control?.isCancelled != true else { throw CancellationError() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git"] + arguments
        process.currentDirectoryURL = directory
        var environment = Self.repositoryEnvironment(ProcessInfo.processInfo.environment)
        environment["PATH"] = pathEnvironment
        environment["GIT_TERMINAL_PROMPT"] = "0"
        environment["GIT_EDITOR"] = "true"
        environment["GIT_SEQUENCE_EDITOR"] = "true"
        // Stash invokes Git internally with its own pathspecs; do not override those.
        if let command = arguments.first, ["add", "clean", "diff", "show", "diff-tree", "restore", "rm", "checkout"].contains(command) {
            environment["GIT_LITERAL_PATHSPECS"] = "1"
        }
        process.environment = environment

        // File-backed output cannot fill a pipe while Git is still running.
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let outputURL = temporary.appendingPathComponent("stdout")
        let errorURL = temporary.appendingPathComponent("stderr")
        FileManager.default.createFile(atPath: outputURL.path, contents: nil)
        FileManager.default.createFile(atPath: errorURL.path, contents: nil)
        let output = try FileHandle(forWritingTo: outputURL)
        let errorOutput = try FileHandle(forWritingTo: errorURL)
        defer { try? output.close(); try? errorOutput.close() }
        process.standardOutput = output
        process.standardError = errorOutput
        process.standardInput = FileHandle.nullDevice

        do {
            try process.run()
            try GitProcessWaiter.wait(process, control: control, timeout: commandTimeout)
        } catch {
            throw GitClientError.commandFailed(command: "git \(arguments.joined(separator: " "))", message: error.localizedDescription)
        }

        let outputData = try Data(contentsOf: outputURL)
        let errorData = try Data(contentsOf: errorURL)
        let outputText = String(data: outputData, encoding: .utf8)
        let errorText = String(data: errorData, encoding: .utf8) ?? ""

        guard acceptedStatuses.contains(process.terminationStatus) else {
            let message = errorText.trimmingCharacters(in: .whitespacesAndNewlines)
            throw GitClientError.commandFailed(
                command: "git \(arguments.joined(separator: " "))",
                message: message.isEmpty ? "Exited with status \(process.terminationStatus)." : message
            )
        }

        guard let outputText else {
            throw GitClientError.commandFailed(command: "git \(arguments.joined(separator: " "))", message: "Git returned non-UTF-8 data that cannot be displayed as text.")
        }
        return outputText
    }
}
