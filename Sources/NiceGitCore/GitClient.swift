import Foundation

public struct GitClient: Sendable {
    private let pathEnvironment = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

    private let control: GitCommandControl?
    private let commandTimeout: TimeInterval
    /// Lets `git status` save refreshed file information to the index, as it normally would.
    /// Only deliberate loads use this; automatic refreshes leave the index untouched so they
    /// never contend for its lock with the user's own Git commands.
    private let statusUpdatesIndex: Bool

    public init(control: GitCommandControl? = nil, commandTimeout: TimeInterval = 600, statusUpdatesIndex: Bool = false) {
        self.control = control
        self.commandTimeout = commandTimeout
        self.statusUpdatesIndex = statusUpdatesIndex
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
        guard try loadStatus(in: url).contains(where: { $0.path == path && $0.kind == .conflicted }) else {
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

    /// Applies a commit identity to this repository only. With a signing key, commits are also
    /// signed with it; without one, the repository's signing settings are left unchanged.
    public func applyIdentity(name: String, email: String, signingKey: String?, in url: URL) throws {
        try setIdentity(name: name, email: email, in: url)
        if let signingKey = signingKey?.trimmingCharacters(in: .whitespacesAndNewlines), !signingKey.isEmpty {
            try run(["config", "--local", "user.signingkey", signingKey], in: url)
            try run(["config", "--local", "commit.gpgsign", "true"], in: url)
        }
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

    /// Renames a remote; Git also moves its remote-tracking branches and branch upstream settings.
    public func renameRemote(_ name: String, to newName: String, expectedAddress: String?, in url: URL) throws {
        try requireRemoteAddress(name, expectedAddress: expectedAddress, in: url)
        try run(["remote", "rename", "--", name, newName], in: url)
    }

    /// Removes a remote with its remote-tracking branches and the upstream settings that use it.
    public func removeRemote(_ name: String, expectedAddress: String?, in url: URL) throws {
        try requireRemoteAddress(name, expectedAddress: expectedAddress, in: url)
        try run(["remote", "remove", "--", name], in: url)
    }

    /// Changes a remote's fetch address. Separately configured push addresses stay as they are.
    public func setRemoteAddress(_ name: String, to address: String, expectedAddress: String?, in url: URL) throws {
        guard !address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw GitClientError.commandFailed(command: "remote set-url", message: "Enter the remote's new address.")
        }
        try requireRemoteAddress(name, expectedAddress: expectedAddress, in: url)
        try run(["remote", "set-url", "--", name, address], in: url)
    }

    /// Rejects an action confirmed for a remote whose address changed after it was shown.
    private func requireRemoteAddress(_ name: String, expectedAddress: String?, in url: URL) throws {
        let current = try? remoteAddress(name: name, in: url)
        guard let current, current == expectedAddress else {
            throw GitClientError.commandFailed(command: "remote", message: "The remote \(name) changed or was removed since it was shown. Refresh and review it again.")
        }
    }

    /// Cherry-picks several commits onto the current branch, oldest first by commit time. Merge
    /// commits are refused because each needs a chosen parent. Conflicts stop the sequence for
    /// Continue or Abort, like a single cherry-pick.
    public func cherryPick(_ commits: [String], expectedHead: String?, expectedBranch: String, in url: URL) throws {
        let snapshot = try checkoutState(in: url, includingStatus: true)
        guard snapshot.headHash == expectedHead, snapshot.currentBranch == expectedBranch else {
            throw GitClientError.commandFailed(command: "cherry-pick", message: "The current branch or HEAD changed since this action was selected. Refresh and review the operation again.")
        }
        guard snapshot.operation == nil, snapshot.status.isEmpty else {
            throw GitClientError.commandFailed(command: "cherry-pick", message: "Commit or stash changes and finish the current operation first.")
        }
        var resolved: [(hash: String, time: Int)] = []
        for commit in Set(commits) {
            let fields = try run(["show", "--no-patch", "--format=%H %ct %P", "--end-of-options", commit + "^{commit}", "--"], in: url)
                .trimmingCharacters(in: .whitespacesAndNewlines).split(separator: " ")
            guard fields.count <= 3 else {
                throw GitClientError.commandFailed(command: "cherry-pick", message: "Merge commits cannot be cherry-picked together with others. Pick a merge on its own and choose its parent.")
            }
            resolved.append((String(fields[0]), Int(fields[1]) ?? 0))
        }
        guard !resolved.isEmpty else { return }
        try run(["cherry-pick"] + resolved.sorted { ($0.time, $0.hash) < ($1.time, $1.hash) }.map(\.hash), in: url)
    }

    public func start(_ operation: GitOperation, target: String, mainline: Int? = nil, expectedHead: String? = nil, expectedBranch: String? = nil, expectedSourceBranch: GitBranch? = nil, in url: URL) throws {
        let snapshot = try checkoutState(in: url, includingStatus: true)
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
        if operation == .merge {
            if snapshot.headHash != nil { try requireNoIgnoredMergeCollisions(target: hash, in: url) }
            arguments.append("--no-overwrite-ignore")
        }
        if operation == .merge || operation == .revert { arguments.append("--no-edit") }
        if let mainline, operation == .revert || operation == .cherryPick { arguments += ["--mainline", String(mainline)] }
        try run(arguments + [hash], in: url)
    }

    func requireNoIgnoredMergeCollisions(target: String, in url: URL) throws {
        let bases = try run(["merge-base", "--all", "HEAD", target], in: url, acceptedStatuses: [0, 1])
            .split(whereSeparator: \.isWhitespace)
        var candidates = Set<String>()
        for base in bases {
            let paths = try run(["diff", "--name-only", "--no-renames", "--diff-filter=ACMRT", "-z", String(base), target, "--"], in: url)
                .split(separator: "\0")
            for path in paths {
                var prefix = ""
                let components = path.split(separator: "/")
                for (index, component) in components.enumerated() {
                    prefix += (prefix.isEmpty ? "" : "/") + component
                    guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.appendingPathComponent(prefix).path) else { break }
                    if attributes[.type] as? FileAttributeType != .typeDirectory || index == components.count - 1 {
                        candidates.insert(prefix)
                        break
                    }
                }
            }
        }
        guard !candidates.isEmpty else { return }
        let ignored = try run(["ls-files", "--others", "--ignored", "--exclude-standard", "-z", "--"] + candidates.sorted(), in: url)
        guard ignored.isEmpty else {
            throw GitClientError.commandFailed(command: "merge", message: "Ignored local files would be overwritten by this merge. Move or back them up before merging.")
        }
    }

    public func reset(to target: String, mode: GitResetMode, expectedHead: String, expectedBranch: String, in url: URL) throws {
        let (snapshot, hash) = try checkoutState(in: url, resolving: target)
        guard snapshot.operation == nil, snapshot.headHash == expectedHead, snapshot.currentBranch == expectedBranch else {
            throw GitClientError.commandFailed(command: "reset", message: "The checkout changed or a Git operation is in progress. Refresh and review the reset again.")
        }
        try run(["reset", "--" + mode.rawValue, hash, "--"], in: url)
    }

    public func continueOperation(_ operation: GitOperation, in url: URL) throws {
        try run([operation.rawValue, "--continue"], in: url)
    }

    public func abortOperation(_ operation: GitOperation, in url: URL) throws {
        try run([operation.rawValue, "--abort"], in: url)
    }

    public func currentOperation(in url: URL) throws -> GitOperation? {
        try currentOperation(gitDirectory: gitDirectory(in: url))
    }

    /// This checkout's own Git directory (per worktree), as an absolute path. Remembered per
    /// checkout while it still holds a HEAD file, so routine refreshes need not ask Git again.
    func gitDirectory(in url: URL) throws -> URL {
        if let cached = GitDirectoryCache.shared[url.path],
           FileManager.default.fileExists(atPath: cached.appendingPathComponent("HEAD").path) { return cached }
        var gitDirectory = try run(["rev-parse", "--absolute-git-dir"], in: url)
        if gitDirectory.hasSuffix("\n") { gitDirectory.removeLast() }
        let directory = URL(fileURLWithPath: gitDirectory, isDirectory: true)
        GitDirectoryCache.shared[url.path] = directory
        return directory
    }

    func currentOperation(gitDirectory directory: URL) throws -> GitOperation? {
        let markers: [(String, GitOperation)] = [("rebase-merge", .rebase), ("rebase-apply", .rebase), ("MERGE_HEAD", .merge), ("CHERRY_PICK_HEAD", .cherryPick), ("REVERT_HEAD", .revert)]
        for (marker, operation) in markers {
            if FileManager.default.fileExists(atPath: directory.appendingPathComponent(marker).path) { return operation }
        }
        let todo = directory.appendingPathComponent("sequencer/todo")
        if FileManager.default.fileExists(atPath: todo.path) {
            for line in try String(contentsOf: todo, encoding: .utf8).split(separator: "\n") {
                switch line.split(whereSeparator: \.isWhitespace).first {
                case "pick": return .cherryPick
                case "revert": return .revert
                default: continue
                }
            }
        }
        return nil
    }

    public func saveStash(message: String, includeUntracked: Bool, in url: URL) throws {
        try requireFinishedOperation(command: "stash", in: url)
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

    /// Stashes only `paths`, staged and unstaged, including selected untracked files. Every other
    /// file is left exactly as it was.
    public func saveStash(paths: [String], message: String, in url: URL) throws {
        guard !paths.isEmpty else { throw GitClientError.commandFailed(command: "stash", message: "Select files to stash.") }
        try requireFinishedOperation(command: "stash", in: url)
        let before = try loadStatus(in: url)
        let selected = Set(paths)
        guard selected.allSatisfy({ path in before.contains { $0.path == path } }) else {
            throw GitClientError.commandFailed(command: "stash", message: "Some selected files changed since they were selected. Refresh and review them again.")
        }
        let previous = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?.trimmingCharacters(in: .whitespacesAndNewlines)
        let stashMessage = message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "WIP on \(currentBranch(in: url))" : message
        // Stash runs Git internally with its own pathspecs, so mark each path literal here rather
        // than through the environment; a glob-like name must not select other files.
        try run(["stash", "push", "--include-untracked", "-m", stashMessage, "--"] + paths.map { ":(literal)" + $0 }, in: url)
        guard let saved = (try? run(["rev-parse", "--verify", "refs/stash"], in: url))?.trimmingCharacters(in: .whitespacesAndNewlines), saved != previous else {
            throw GitClientError.commandFailed(command: "stash", message: "Git did not save any changes for the selected files.")
        }
        let after = try loadStatus(in: url)
        if after.contains(where: { selected.contains($0.path) }) {
            throw GitClientError.commandFailed(command: "stash", message: "Stash \(saved.prefix(12)) was saved, but some selected files still have changes. Check submodules before proceeding.")
        }
        let others = { (entries: [GitStatusEntry]) in entries.filter { !selected.contains($0.path) } }
        if others(after) != others(before) {
            throw GitClientError.commandFailed(command: "stash", message: "Stash \(saved.prefix(12)) was saved, but other files changed too. Review the working tree and the stash before continuing.")
        }
    }

    private func pushStash(message: String, includeUntracked: Bool, in url: URL) throws {
        let stashMessage = message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "WIP on \(currentBranch(in: url))" : message
        try run(["stash", "push"] + (includeUntracked ? ["--include-untracked"] : []) + ["-m", stashMessage], in: url)
    }

    public func applyStash(_ stash: GitStash, in url: URL) throws {
        try requireFinishedOperation(command: "stash apply", in: url)
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

    public func diff(path: String, staged: Bool, untracked: Bool = false, originalPath: String? = nil, ignoreWhitespace: Bool = false, in repositoryURL: URL) throws -> String {
        if untracked {
            return try run(["diff", "--no-index", "--no-ext-diff", "--no-color", "--", "/dev/null", path], in: repositoryURL, acceptedStatuses: [0, 1])
        }
        var renameSource: String?
        if let originalPath {
            guard let entry = try loadStatus(in: repositoryURL).first(where: { $0.path == path && $0.originalPath == originalPath }) else {
                throw GitClientError.commandFailed(command: "diff", message: "This file changed since it was selected. Refresh and review it again.")
            }
            if entry.kind == .renamed { renameSource = originalPath }
        }
        return try run(["diff", "--no-ext-diff", "--no-color"] + (ignoreWhitespace ? ["-w"] : []) + (staged ? ["--cached"] : []) + ["--", path] + (renameSource.map { [$0] } ?? []), in: repositoryURL)
    }

    public func commitDiff(hash: String, path: String? = nil, ignoreWhitespace: Bool = false, in repositoryURL: URL) throws -> String {
        try run(["show", "--first-parent", "-m", "--format=fuller", "--stat", "--patch", "--no-ext-diff", "--no-color"] + (ignoreWhitespace ? ["-w"] : []) + [hash, "--"] + (path.map { [$0] } ?? []), in: repositoryURL)
    }

    public func commitFileDiff(hash: String, path: String, ignoreWhitespace: Bool = false, in repositoryURL: URL) throws -> String {
        try run(["show", "--first-parent", "-m", "--format=", "--patch", "--unified=3", "--no-renames", "--no-ext-diff", "--no-color"] + (ignoreWhitespace ? ["-w"] : []) + [hash, "--", path], in: repositoryURL)
    }

    /// Commits in the current checkout's history that changed `path`, newest first, following renames.
    public func fileHistory(path: String, limit: Int = 200, in repositoryURL: URL) throws -> [GitFileHistoryEntry] {
        guard (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) != nil else { return [] }
        // Each record is the commit fields, then NUL-separated change letter and path; renames
        // and copies list the old path before the new one.
        let format = "%x1e%H%x1f%h%x1f%P%x1f%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct"
        let output = try run(["log", "--follow", "--no-color", "-z", "--name-status", "-n", String(max(1, limit)),
                              "--format=" + format, "HEAD", "--", path], in: repositoryURL)
        // Split only on NUL: paths may contain any other byte, including Git's field separators.
        var entries: [GitFileHistoryEntry] = []
        var tokens = output.split(separator: "\0", omittingEmptySubsequences: false)[...]
        while let token = tokens.popFirst() {
            guard token.hasPrefix("\u{1e}"), let commit = GitLogParser.parse(String(token.dropFirst())).first else { continue }
            // Git separates the fields from the change letter with one newline.
            guard let statusToken = tokens.first, statusToken.hasPrefix("\n") else {
                entries.append(GitFileHistoryEntry(commit: commit, path: path, status: "M"))
                continue
            }
            tokens.removeFirst()
            let status = String(statusToken.dropFirst())
            let pathCount = status.hasPrefix("R") || status.hasPrefix("C") ? 2 : 1
            let paths = tokens.prefix(pathCount)
            tokens.removeFirst(paths.count)
            entries.append(GitFileHistoryEntry(commit: commit, path: paths.last.map(String.init) ?? path, status: String(status.prefix(1))))
        }
        return entries
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

    public func stashDiff(hash: String, ignoreWhitespace: Bool = false, in repositoryURL: URL) throws -> String {
        try run(["stash", "show", "--include-untracked", "--patch", "--stat", "--no-ext-diff", "--no-color"] + (ignoreWhitespace ? ["-w"] : []) + [hash], in: repositoryURL)
    }

    public func stashFiles(hash: String, in repositoryURL: URL) throws -> [String] {
        try run(["stash", "show", "--include-untracked", "--name-only", "-z", hash], in: repositoryURL)
            .split(separator: "\0").map(String.init).sorted()
    }

    /// Reads the repository's state. With `reusing`, a snapshot of the same repository taken just
    /// before an action that cannot change remotes, worktrees, tags, or stashes (such as a commit
    /// or reset), those are carried over instead of read again.
    public func loadSnapshot(at selectedURL: URL, historyLimit: Int = 200, reusing previous: RepositorySnapshot? = nil) throws -> RepositorySnapshot {
        let rootPath = try knownRepositoryRoot(selectedURL) ?? repositoryRoot(for: selectedURL)
        let rootURL = URL(fileURLWithPath: rootPath)
        let limit = max(1, historyLimit)
        let logFormat = "%H%x1f%h%x1f%P%x1f%(decorate:prefix=,suffix=,separator=%x1d,pointer=%x1c,tag=tag: )%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e"
        // These reads are independent, so run them together: a refresh then takes about as long
        // as its slowest command rather than the sum of all of them. `--all` includes HEAD.
        let reused = previous?.rootPath == rootPath ? previous : nil
        // A Git directory already found for this checkout needs no `rev-parse`.
        let cachedGitDirectory = GitDirectoryCache.shared[rootURL.path].flatMap {
            FileManager.default.fileExists(atPath: $0.appendingPathComponent("HEAD").path) ? $0 : nil
        }
        // Index 3-5 are skipped (left empty) when reusing; their results come from `reused`.
        let skipped: [[String]] = reused == nil ? [] : [[], [], []]
        let reads = try runConcurrently([
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            ["branch", "--all", "--format=" + GitBranchParser.format],
            ["log", "--exclude=refs/stash", "--all", "--topo-order", "--decorate=short", "--date=relative",
             "-n", String(limit + 1), "--pretty=format:" + logFormat, "--"],
        ] + (reused == nil ? [
            ["remote", "-v"],
            ["worktree", "list", "--porcelain", "-z"],
            ["for-each-ref", "--sort=-version:refname", "--format=%(refname:strip=2)%09%(objectname)", "refs/tags"],
        ] : skipped) + [
            ["rev-parse", "--absolute-git-dir"],
            // Git resolves the current branch's upstream itself, so this need not wait for the
            // branch listing; it fails harmlessly when there is no upstream.
            ["rev-list", "--left-right", "--count", "HEAD...@{upstream}", "--"],
        ], in: rootURL, optional: [7], skipping: Set(reused == nil ? [] : [3, 4, 5]).union(cachedGitDirectory == nil ? [] : [6]),
           alongside: { try reused?.stashes ?? self.listStashes(in: rootURL) })
        let status = try GitStatusParser.parseNullTerminated(reads.outputs[0])
        let branches = GitBranchParser.parse(reads.outputs[1], includesSymref: true)
        let current = branches.first(where: \.isCurrent)
        let branch = current.flatMap { $0.name.hasPrefix("(") ? nil : $0.name } ?? currentBranch(in: rootURL)
        let headHash = current?.tip
        let commits = GitLogParser.parse(reads.outputs[2])
        var visibleCommits = Array(commits.prefix(limit))
        if let headHash, !visibleCommits.contains(where: { $0.hash == headHash }) {
            if let headCommit = commits.first(where: { $0.hash == headHash }) {
                visibleCommits.append(headCommit)
            } else {
                let headCommit = GitLogParser.parse(try run(["log", "-1", "--decorate=short", "--pretty=format:" + logFormat, "HEAD", "--"], in: rootURL))
                visibleCommits.append(contentsOf: headCommit.prefix(1))
            }
        }
        let remoteOutput = reads.outputs[3]
        let reusedRemotes = reused.map { ($0.remotes, $0.remoteAddresses, $0.remoteFetchAddresses, $0.remotePushAddresses) }
        let remotes = GitRemoteParser.parse(remoteOutput)

        var snapshot = RepositorySnapshot(
            rootPath: rootPath,
            name: rootURL.lastPathComponent,
            currentBranch: branch,
            status: status,
            branches: branches,
            commits: visibleCommits,
            remotes: remotes
        )
        snapshot.remoteAddresses = GitRemoteParser.addresses(remoteOutput)
        snapshot.remoteFetchAddresses = GitRemoteParser.allAddresses(remoteOutput, direction: "fetch")
        snapshot.remotePushAddresses = GitRemoteParser.allAddresses(remoteOutput, direction: "push")
        if let reusedRemotes {
            (snapshot.remotes, snapshot.remoteAddresses, snapshot.remoteFetchAddresses, snapshot.remotePushAddresses) = reusedRemotes
        }
        snapshot.stashes = try reads.alongside.get()
        snapshot.headHash = headHash
        snapshot.worktrees = reused?.worktrees ?? GitWorktree.parse(reads.outputs[4])
        if let reused {
            snapshot.tags = reused.tags
            snapshot.tagTips = reused.tagTips
        }
        let tagLines = reused == nil ? reads.outputs[5] : ""
        for line in tagLines.split(separator: "\n") {
            let parts = line.split(separator: "\t", maxSplits: 1)
            guard parts.count == 2 else { continue }
            let name = String(parts[0])
            snapshot.tags.append(name)
            snapshot.tagTips[name] = String(parts[1])
        }
        if let upstream = current?.upstream, !reads.outputs[7].isEmpty {
            let counts = reads.outputs[7]
            let values = counts.split(whereSeparator: { $0.isWhitespace }).compactMap { Int($0) }
            snapshot.upstream = upstream.hasPrefix("refs/remotes/") ? String(upstream.dropFirst("refs/remotes/".count)) :
                (upstream.hasPrefix("refs/heads/") ? String(upstream.dropFirst("refs/heads/".count)) : upstream)
            if values.count == 2 {
                snapshot.ahead = values[0]
                snapshot.behind = values[1]
            }
        }
        snapshot.hasMoreCommits = commits.count > limit
        var gitPath = reads.outputs[6]
        // Paths can contain newlines; remove only Git's final line terminator.
        if gitPath.hasSuffix("\n") { gitPath.removeLast() }
        let gitDirectory = cachedGitDirectory ?? URL(fileURLWithPath: gitPath, isDirectory: true)
        GitDirectoryCache.shared[rootURL.path] = gitDirectory
        snapshot.operation = try currentOperation(gitDirectory: gitDirectory)
        // Only read bisect details while one is running; most refreshes check a single file.
        if FileManager.default.fileExists(atPath: gitDirectory.appendingPathComponent("BISECT_START").path) {
            snapshot.bisect = try? bisectStatus(gitDirectory: gitDirectory, in: rootURL)
        }
        return snapshot
    }

    /// Runs independent read-only commands at the same time, plus one extra piece of work,
    /// returning outputs in order. The first failure is rethrown after all have finished.
    func runConcurrently<Extra: Sendable>(_ commands: [[String]], in url: URL, optional: Set<Int> = [], skipping: Set<Int> = [],
                                          alongside: @escaping @Sendable () throws -> Extra) throws -> (outputs: [String], alongside: Result<Extra, Error>) {
        let results = ConcurrentResults<Extra>(count: commands.count)
        // Starting every process at once makes each slower; a few at a time finishes sooner.
        let slots = DispatchSemaphore(value: Self.concurrentCommandLimit)
        DispatchQueue.concurrentPerform(iterations: commands.count + 1) { index in
            slots.wait()
            defer { slots.signal() }
            if index == commands.count {
                let value = Result { try alongside() }
                results.lock.lock(); results.extra = value; results.lock.unlock()
            } else {
                // Skipped slots keep their position so callers can index results; they stay empty.
                let value = skipping.contains(index) ? .success("") : Result { try self.run(commands[index], in: url) }
                results.lock.lock(); results.outputs[index] = value; results.lock.unlock()
            }
        }
        // Commands listed in `optional` may fail; their output is then empty.
        return (try results.outputs.enumerated().map { index, result in optional.contains(index) ? ((try? result!.get()) ?? "") : try result!.get() }, results.extra!)
    }

    /// The checkout's branch, HEAD, unfinished operation, and optionally its file status, read
    /// together. Actions use this to confirm the checkout they were chosen for; a full snapshot
    /// would read far more than they need.
    public struct CheckoutState: Sendable {
        public let currentBranch: String
        public let headHash: String?
        public let operation: GitOperation?
        public let status: [GitStatusEntry]
    }

    public func checkoutState(in url: URL, includingStatus: Bool = false) throws -> CheckoutState {
        let reads = try runConcurrently(
            [["branch", "--show-current"], ["rev-parse", "--verify", "--quiet", "HEAD"]]
                + (includingStatus ? [["status", "--porcelain=v1", "-z", "--untracked-files=all"]] : []),
            in: url, optional: [1], alongside: { try self.currentOperation(in: url) })
        let head = reads.outputs[1].trimmingCharacters(in: .whitespacesAndNewlines)
        var branch = reads.outputs[0].trimmingCharacters(in: .whitespacesAndNewlines)
        // A detached HEAD has no branch name; describe it as the snapshot does.
        if branch.isEmpty { branch = currentBranch(in: url) }
        return CheckoutState(currentBranch: branch, headHash: head.isEmpty ? nil : head, operation: try reads.alongside.get(),
                             status: includingStatus ? try GitStatusParser.parseNullTerminated(reads.outputs[2]) : [])
    }

    /// The checkout state, and `revision` resolved to a commit, read at the same time.
    func checkoutState(in url: URL, resolving revision: String) throws -> (CheckoutState, String) {
        var state: Result<CheckoutState, Error> = .success(CheckoutState(currentBranch: "", headHash: nil, operation: nil, status: []))
        var commit: Result<String, Error> = .success("")
        inParallel(
            { state = Result { try self.checkoutState(in: url) } },
            { commit = Result { try self.run(["rev-parse", "--verify", "--end-of-options", revision + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines) } })
        return (try state.get(), try commit.get())
    }

    /// The repository root without running Git, when `url` itself holds the `.git` entry of a
    /// checkout, linked worktree, or submodule. Resolved like Git's own `--show-toplevel`.
    private func knownRepositoryRoot(_ url: URL) throws -> String? {
        guard (try? FileManager.default.attributesOfItem(atPath: url.appendingPathComponent(".git").path)) != nil,
              let resolved = realpath(url.path, nil) else { return nil }
        defer { free(resolved) }
        return String(cString: resolved)
    }

    /// Status for just these paths, for confirming a selected file has not changed. Cheaper
    /// than a full status, which also walks every other directory for untracked files.
    func loadStatus(in repositoryURL: URL, paths: [String]) throws -> [GitStatusEntry] {
        try GitStatusParser.parseNullTerminated(run(["status", "--porcelain=v1", "-z", "--untracked-files=all", "--"] + paths, in: repositoryURL))
    }

    public func loadStatus(in repositoryURL: URL) throws -> [GitStatusEntry] {
        try GitStatusParser.parseNullTerminated(run(["status", "--porcelain=v1", "-z", "--untracked-files=all"], in: repositoryURL))
    }

    public func loadStatusWithCheckout(in repositoryURL: URL) throws -> (entries: [GitStatusEntry], branch: String?, headHash: String?, isComplete: Bool) {
        GitStatusParser.parseWithCheckout(try run(["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"], in: repositoryURL))
    }

    /// Status with branch headers and the unfinished operation, read at the same time.
    public func loadStatusWithCheckoutAndOperation(in repositoryURL: URL) throws -> (status: (entries: [GitStatusEntry], branch: String?, headHash: String?, isComplete: Bool), operation: GitOperation?) {
        let reads = try runConcurrently([["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"]], in: repositoryURL,
                                        alongside: { try self.currentOperation(in: repositoryURL) })
        return (GitStatusParser.parseWithCheckout(reads.outputs[0]), try reads.alongside.get())
    }

    public func stage(path: String, in repositoryURL: URL) throws {
        try run(["add", "--", path], in: repositoryURL)
    }

    public func stageAll(in repositoryURL: URL) throws {
        try run(["add", "--all"], in: repositoryURL)
    }

    /// With `headKnownToExist`, the usual restore runs straight away; the check for a repository
    /// without commits happens only if that restore fails.
    public func unstage(path: String, originalPath: String? = nil, headKnownToExist: Bool = false, in repositoryURL: URL) throws {
        var renameSource: String?
        if let originalPath {
            guard let entry = try loadStatus(in: repositoryURL).first(where: { $0.path == path && $0.originalPath == originalPath }) else {
                throw GitClientError.commandFailed(command: "unstage", message: "This file changed since it was selected. Refresh and review it again.")
            }
            if entry.kind == .renamed { renameSource = originalPath }
        }
        let restore = ["restore", "--staged", "--", path] + (renameSource.map { [$0] } ?? [])
        if headKnownToExist, (try? run(restore, in: repositoryURL)) != nil { return }
        if (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) == nil {
            try run(["rm", "--cached", "--force", "--", path], in: repositoryURL)
        } else {
            try run(restore, in: repositoryURL)
        }
    }

    public func unstageAll(in repositoryURL: URL) throws {
        if (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) == nil {
            try run(["rm", "--cached", "--force", "-r", "--", "."], in: repositoryURL)
        } else {
            try run(["restore", "--staged", "."], in: repositoryURL)
        }
    }

    public func discard(_ entry: GitStatusEntry, in repositoryURL: URL) throws {
        var status: Result<[GitStatusEntry], Error> = .success([])
        var hasHead = false
        inParallel(
            { status = Result { try self.loadStatus(in: repositoryURL, paths: [entry.path] + [entry.originalPath].compactMap { $0 }) } },
            { hasHead = (try? self.run(["rev-parse", "--verify", "HEAD"], in: repositoryURL)) != nil })
        try discard(entry, statusBefore: status.get(), hasHead: hasHead, in: repositoryURL)
        try verifyDiscard(entry, statusAfter: loadStatus(in: repositoryURL))
    }

    /// Runs the discard itself once the caller has confirmed the status still lists `entry`.
    func discard(_ entry: GitStatusEntry, statusBefore: [GitStatusEntry], hasHead: Bool, in repositoryURL: URL) throws {
        guard statusBefore.contains(entry) else {
            throw GitClientError.commandFailed(command: "discard", message: "This file changed since it was selected. Refresh and review it again.")
        }
        let renameSource = entry.kind == .renamed ? entry.originalPath : nil
        if entry.kind == .untracked {
            try run(["clean", "--force", "--", entry.path], in: repositoryURL)
        } else if !hasHead {
            try run(["rm", "--force", "--", entry.path], in: repositoryURL)
        } else {
            try run(["restore", "--source=HEAD", "--staged", "--worktree", "--", entry.path] + (renameSource.map { [$0] } ?? []), in: repositoryURL)
        }
    }

    /// Git can report success while changes remain, for example in a dirty submodule.
    func verifyDiscard(_ entry: GitStatusEntry, statusAfter: [GitStatusEntry]) throws {
        let renameSource = entry.kind == .renamed ? entry.originalPath : nil
        if statusAfter.contains(where: { $0.path == entry.path || $0.path == renameSource }) {
            let message = entry.kind == .untracked
                ? "Git could not remove this untracked path. Nested repositories require manual removal."
                : "Changes remain after Git restored this path. If it is a submodule, open it and discard its changes there."
            throw GitClientError.commandFailed(command: "discard", message: message)
        }
    }

    /// Runs independent pieces of work at the same time and returns when all have finished.
    func inParallel(_ work: (() -> Void)...) {
        let tasks = work
        DispatchQueue.concurrentPerform(iterations: tasks.count) { tasks[$0]() }
    }

    /// Restores one path's staged and working copies to its version in `source`, removing it when
    /// that version has no such file. Staged and unstaged edits to the path are replaced.
    public func restore(path: String, from source: String, expectedBranch: String, expectedHead: String?, in url: URL) throws {
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: "restore file", in: url)
        guard try currentOperation(in: url) == nil else {
            throw GitClientError.commandFailed(command: "restore file", message: "Finish or abort the current Git operation before restoring files.")
        }
        let commit = try run(["rev-parse", "--verify", "--end-of-options", source + "^{commit}"], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        // Both listings end each record with NUL and separate metadata from the path with a tab.
        func entry(_ output: String) -> [Substring]? {
            output.split(separator: "\0").first { $0.split(separator: "\t", maxSplits: 1).last == Substring(path) }?
                .split(separator: "\t", maxSplits: 1).first?.split(separator: " ")
        }
        let sourceEntry = entry(try run(["ls-tree", "-z", commit, "--", path], in: url))
        let indexEntries = try run(["ls-files", "-z", "--stage", "--", path], in: url)
            .split(separator: "\0").filter { $0.split(separator: "\t", maxSplits: 1).last == Substring(path) }
        if indexEntries.contains(where: { $0.split(separator: " ").dropFirst(2).first.map { !$0.hasPrefix("0") } ?? false }) {
            throw GitClientError.commandFailed(command: "restore file", message: "This file has unresolved conflicts. Resolve them before restoring it.")
        }
        if sourceEntry?.first == "160000" || indexEntries.contains(where: { $0.hasPrefix("160000 ") }) {
            throw GitClientError.commandFailed(command: "restore file", message: "Submodules cannot be restored here. Check out the wanted commit inside the submodule instead.")
        }
        if let type = sourceEntry?.dropFirst().first, type != "blob" {
            throw GitClientError.commandFailed(command: "restore file", message: "This path is a folder in the selected commit. Choose an individual file.")
        }
        let file = url.appendingPathComponent(path)
        let exists = (try? FileManager.default.attributesOfItem(atPath: file.path)) != nil
        if indexEntries.isEmpty && exists {
            // Git would overwrite or refuse a local file it does not track; never replace one silently.
            throw GitClientError.commandFailed(command: "restore file", message: "An untracked file or a folder is at this path in your working tree. Move or rename it before restoring.")
        }
        guard sourceEntry != nil || !indexEntries.isEmpty else { return }
        try run(["restore", "--source=" + commit, "--staged", "--worktree", "--", path], in: url)
        let matches = (try? run(["diff", "--quiet", "--no-ext-diff", commit, "--", path], in: url)) != nil
            && (try? run(["diff", "--quiet", "--no-ext-diff", "--cached", commit, "--", path], in: url)) != nil
            && (sourceEntry != nil || (try? FileManager.default.attributesOfItem(atPath: file.path)) == nil)
        guard matches else {
            throw GitClientError.commandFailed(command: "restore file", message: "Git restored this path, but it still differs from the selected commit. Refresh and review it.")
        }
    }

    public func commit(message: String, in repositoryURL: URL) throws {
        let trimmedMessage = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedMessage.isEmpty else {
            throw GitClientError.emptyCommitMessage
        }

        // --quiet skips the summary and its diffstat, which NiceGit does not show.
        try run(["commit", "--quiet", "-m", trimmedMessage], in: repositoryURL)
    }

    @discardableResult
    public func checkout(branch: String, expectedTip: String? = nil, expectedCurrentBranch: String? = nil, expectedHead: String? = nil, in repositoryURL: URL) throws -> Bool {
        try requireSelectedCheckout(branch: expectedCurrentBranch, head: expectedHead, command: "switch branch", in: repositoryURL)
        if let expectedTip { try requireBranchTip(branch, expectedTip: expectedTip, in: repositoryURL) }
        return try switchPreservingChanges(["switch", "--no-overwrite-ignore", "--", branch], to: branch, in: repositoryURL)
    }

    /// Replaces the last commit with one that also contains the staged changes and uses `message`.
    public func amendCommit(message: String, expectedBranch: String, expectedHead: String, in url: URL) throws {
        guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw GitClientError.emptyCommitMessage }
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: "amend commit", in: url)
        guard try currentOperation(in: url) == nil else {
            throw GitClientError.commandFailed(command: "amend commit", message: "Finish or abort the current Git operation before amending.")
        }
        try run(["commit", "--amend", "--message", message], in: url)
    }

    /// Whether a remote-tracking branch already contains `commit`.
    public func isPublished(_ commit: String, in url: URL) throws -> Bool {
        let id = try run(["rev-parse", "--verify", "--end-of-options", commit + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        return !(try run(["branch", "--remotes", "--contains", id], in: url)).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public func amendMessage(_ message: String, expectedHead: String, in url: URL) throws {
        guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw GitClientError.emptyCommitMessage }
        let snapshot = try checkoutState(in: url)
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

    /// The checkout's Git directory and, for a linked worktree, the shared one, as absolute paths.
    public func gitDirectories(in url: URL) throws -> [String] {
        var directories: [String] = []
        for arguments in [["rev-parse", "--absolute-git-dir"], ["rev-parse", "--path-format=absolute", "--git-common-dir"]] {
            var path = try run(arguments, in: url)
            // Paths can contain newlines; remove only Git's final line terminator.
            if path.hasSuffix("\n") { path.removeLast() }
            if !directories.contains(path) { directories.append(path) }
        }
        return directories
    }

    /// Removes a linked worktree's folder and registration. Git refuses while it has uncommitted
    /// or untracked files, or is locked; the main worktree and this checkout are never removed.
    public func removeWorktree(at path: String, in url: URL) throws {
        let worktrees = GitWorktree.parse(try run(["worktree", "list", "--porcelain", "-z"], in: url))
        guard let index = worktrees.firstIndex(where: { $0.path == path }) else {
            throw GitClientError.commandFailed(command: "worktree remove", message: "This worktree is no longer listed. Refresh the repository.")
        }
        let current = try repositoryRoot(for: url)
        guard index != 0, path != current else {
            throw GitClientError.commandFailed(command: "worktree remove", message: "The main worktree and the checkout you have open cannot be removed.")
        }
        try run(["worktree", "remove", "--", path], in: url)
    }

    /// Forgets worktrees whose folders were deleted outside Git.
    public func pruneWorktrees(in url: URL) throws {
        try run(["worktree", "prune"], in: url)
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
    public func checkoutRemote(branch: String, expectedTip: String? = nil, expectedCurrentBranch: String? = nil, expectedHead: String? = nil, in url: URL) throws -> Bool {
        try requireSelectedCheckout(branch: expectedCurrentBranch, head: expectedHead, command: "switch branch", in: url)
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
        let branches = try GitBranchParser.parse(run(["branch", "--all", "--format=" + GitBranchParser.format], in: url), includesSymref: true)
        guard branches.contains(where: { $0.isRemote && "refs/" + $0.name == reference }) else {
            throw GitClientError.commandFailed(command: "checkout remote branch", message: "This remote branch is no longer available. Refresh and select a branch again.")
        }
        let tracking = branches.filter { !$0.isRemote && $0.upstream == reference }
        if tracking.count > 1 {
            throw GitClientError.commandFailed(command: "checkout remote branch", message: "Several local branches track this remote branch. Choose the desired branch in Local.")
        }
        if let existing = tracking.first {
            return try checkout(branch: existing.name, expectedTip: existing.tip, expectedCurrentBranch: expectedCurrentBranch, expectedHead: expectedHead, in: url)
        } else {
            let remotePath = String(reference.dropFirst("refs/remotes/".count))
            let remotes = try run(["remote"], in: url).split(separator: "\n").map(String.init)
            guard let remote = remotes.filter({ remotePath.hasPrefix($0 + "/") }).max(by: { $0.count < $1.count }) else {
                throw GitClientError.commandFailed(command: "checkout remote branch", message: "This remote is no longer available. Refresh and select a branch again.")
            }
            let preferred = String(remotePath.dropFirst(remote.count + 1))
            let localNames = branches.filter { !$0.isRemote }.map(\.name)
            func available(_ name: String) -> Bool {
                !localNames.contains { $0 == name || $0.hasPrefix(name + "/") || name.hasPrefix($0 + "/") }
                    && (try? run(["check-ref-format", "--branch", name], in: url)) != nil
            }
            var name = preferred
            if !available(name) {
                let flattened = remotePath.replacingOccurrences(of: "/", with: "-")
                let base = (try? run(["check-ref-format", "--branch", flattened], in: url)) == nil
                    ? "remote-" + flattened : flattened
                name = base
                var suffix = 2
                while !available(name) {
                    name = "\(base)-\(suffix)"
                    suffix += 1
                }
            }
            return try switchPreservingChanges(["switch", "--no-overwrite-ignore", "--track", "--create", name, "--", reference], to: reference, in: url)
        }
    }

    func requireSelectedCheckout(branch expectedBranch: String?, head expectedHead: String?, command: String, in url: URL) throws {
        guard let expectedBranch else { return }
        let currentHead = (try? run(["rev-parse", "--verify", "HEAD"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard currentBranch(in: url) == expectedBranch, currentHead == expectedHead else {
            throw GitClientError.commandFailed(command: command, message: "The current checkout changed since this action was selected. Refresh and review it again.")
        }
    }

    private func switchPreservingChanges(_ arguments: [String], to branch: String, in url: URL) throws -> Bool {
        try requireFinishedOperation(command: "switch branch", in: url)
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

    func requireFinishedOperation(command: String, in url: URL) throws {
        guard try currentOperation(in: url) == nil else {
            throw GitClientError.commandFailed(command: command, message: "Finish or abort the current Git operation before changing the working tree.")
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
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: "create branch", in: repositoryURL)
        guard try currentOperation(in: repositoryURL) == nil else {
            throw GitClientError.commandFailed(command: "create branch", message: "Finish or abort the current Git operation before creating and checking out a branch.")
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

    /// Pushes one tag without moving an existing tag of the same name on the remote.
    public func pushTag(_ name: String, to remote: String, expectedTip: String, expectedPushAddresses: [String: [String]], in url: URL) throws {
        let reference = try requireTagTip(name, expectedTip: expectedTip, in: url)
        try requireRemoteAddresses(expectedPushAddresses, remote: remote, push: true, command: "push tag", in: url)
        try run(["-c", "remote." + remote + ".mirror=false", "push", "--no-follow-tags", "--recurse-submodules=no", "--", remote, reference + ":" + reference], in: url)
    }

    /// Deletes a tag from a remote only while the remote tag still matches the local one.
    public func deleteRemoteTag(_ name: String, from remote: String, expectedTip: String, expectedPushAddresses: [String: [String]], in url: URL) throws {
        let reference = try requireTagTip(name, expectedTip: expectedTip, in: url)
        try requireRemoteAddresses(expectedPushAddresses, remote: remote, push: true, command: "delete remote tag", in: url)
        try run(["-c", "remote." + remote + ".mirror=false", "push", "--no-follow-tags", "--recurse-submodules=no",
                 "--force-with-lease=" + reference + ":" + expectedTip, "--", remote, ":" + reference], in: url)
    }

    private func requireTagTip(_ name: String, expectedTip: String, in url: URL) throws -> String {
        let reference = "refs/tags/" + name
        let current = try? run(["rev-parse", "--verify", "--end-of-options", reference], in: url)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard current == expectedTip else {
            throw GitClientError.commandFailed(command: "tag", message: "This tag changed since it was selected. Refresh and review it again.")
        }
        return reference
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
        let startingBranch = currentBranch(in: repositoryURL)
        let startingHead = (try? run(["rev-parse", "--verify", "HEAD"], in: repositoryURL))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if expectedBranch != nil || expectedHead != nil {
            guard (expectedBranch == nil || expectedBranch == startingBranch),
                  (expectedHead == nil || expectedHead == startingHead) else {
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
        let recurse = try run(["config", "--bool", "--get", "submodule.recurse"], in: repositoryURL, acceptedStatuses: [0, 1])
            .trimmingCharacters(in: .whitespacesAndNewlines) == "true"
        let autoStash = try pullAutoStash(in: repositoryURL)
        try run(["fetch"], in: repositoryURL)
        try requireSelectedCheckout(branch: startingBranch, head: startingHead, command: "pull", in: repositoryURL)
        try run(["merge", "--ff-only", "--no-overwrite-ignore", autoStash ? "--autostash" : "--no-autostash", "FETCH_HEAD"], in: repositoryURL)
        if recurse { try run(["submodule", "update", "--recursive", "--checkout"], in: repositoryURL) }
    }

    private func pullAutoStash(in url: URL) throws -> Bool {
        func config(_ key: String, boolean: Bool = false) throws -> String? {
            let value = try run(["config"] + (boolean ? ["--bool"] : []) + ["--get", key], in: url, acceptedStatuses: [0, 1])
            return value.isEmpty ? nil : value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if let override = try config("pull.autostash", boolean: true) { return override == "true" }
        let branch = (try? run(["symbolic-ref", "--quiet", "--short", "HEAD"], in: url))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let branchKey = branch.map { "branch." + $0 + ".rebase" }
        let key: String
        if let branchKey, try config(branchKey) != nil { key = branchKey }
        else { key = "pull.rebase" }
        let rebase = try config(key)
        let rebasing = try ["merges", "m", "interactive", "i"].contains(rebase ?? "")
            || config(key, boolean: true) == "true"
        return try config(rebasing ? "rebase.autostash" : "merge.autostash", boolean: true) == "true"
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
    func run(_ arguments: [String], in directory: URL, acceptedStatuses: Set<Int32> = [0], environmentOverrides: [String: String] = [:]) throws -> String {
        let outputData = try runData(arguments, in: directory, acceptedStatuses: acceptedStatuses, environmentOverrides: environmentOverrides)
        guard let outputText = String(data: outputData, encoding: .utf8) else {
            throw GitClientError.commandFailed(command: "git \(arguments.joined(separator: " "))", message: "Git returned non-UTF-8 data that cannot be displayed as text.")
        }
        return outputText
    }

    /// Runs Git and returns its raw output, for content such as images that is not text.
    func runData(_ arguments: [String], in directory: URL, acceptedStatuses: Set<Int32> = [0], environmentOverrides: [String: String] = [:]) throws -> Data {
        guard control?.isCancelled != true else { throw CancellationError() }
        let process = Process()
        // Launch Git directly when found: going through /usr/bin/env costs an extra program
        // start and PATH search on every command.
        if let git = Self.gitExecutable(searching: pathEnvironment) {
            process.executableURL = git
            process.arguments = arguments
        } else {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = ["git"] + arguments
        }
        process.currentDirectoryURL = directory
        var environment = Self.repositoryEnvironment(ProcessInfo.processInfo.environment)
        environment["PATH"] = pathEnvironment
        environment["GIT_TERMINAL_PROMPT"] = "0"
        environment["GIT_EDITOR"] = "true"
        environment["GIT_SEQUENCE_EDITOR"] = "true"
        // Read-only commands such as status must not rewrite the index: that would contend with
        // the user's own Git commands and wake NiceGit's file watcher in a loop.
        if !(statusUpdatesIndex && arguments.first == "status") { environment["GIT_OPTIONAL_LOCKS"] = "0" }
        // Stash invokes Git internally with its own pathspecs; do not override those.
        if let command = arguments.first, ["add", "clean", "diff", "show", "diff-tree", "restore", "rm", "checkout", "ls-files", "ls-tree", "log", "status"].contains(command) {
            environment["GIT_LITERAL_PATHSPECS"] = "1"
        }
        environment.merge(environmentOverrides) { _, override in override }
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

        let started = GitCommandLog.shared.isRecording ? ContinuousClock.now : nil
        defer {
            if let started {
                let elapsed = ContinuousClock.now - started
                GitCommandLog.shared.record(arguments, seconds: Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18, started: started)
            }
        }
        let finished = GitProcessWaiter.prepare(process)
        do {
            try process.run()
            try GitProcessWaiter.wait(process, finished: finished, control: control, timeout: commandTimeout)
        } catch {
            throw GitClientError.commandFailed(command: "git \(arguments.joined(separator: " "))", message: error.localizedDescription)
        }

        let outputData = try Data(contentsOf: outputURL)
        let errorData = try Data(contentsOf: errorURL)
        let errorText = String(data: errorData, encoding: .utf8) ?? ""

        guard acceptedStatuses.contains(process.terminationStatus) else {
            let message = errorText.trimmingCharacters(in: .whitespacesAndNewlines)
            throw GitClientError.commandFailed(
                command: "git \(arguments.joined(separator: " "))",
                message: message.isEmpty ? "Exited with status \(process.terminationStatus)." : message
            )
        }

        return outputData
    }
}

/// Collects results from concurrent Git reads.
private final class ConcurrentResults<Extra>: @unchecked Sendable {
    let lock = NSLock()
    var outputs: [Result<String, Error>?]
    var extra: Result<Extra, Error>?
    init(count: Int) { outputs = Array(repeating: nil, count: count) }
}

/// Git directories already found, by checkout path.
final class GitDirectoryCache: @unchecked Sendable {
    static let shared = GitDirectoryCache()
    private let lock = NSLock()
    private var directories: [String: URL] = [:]

    subscript(path: String) -> URL? {
        get { lock.lock(); defer { lock.unlock() }; return directories[path] }
        set { lock.lock(); directories[path] = newValue; lock.unlock() }
    }
}

extension GitClient {
    /// The first executable `git` in `path`, in the same order `env` would search. Found once;
    /// a missing Git is looked for again next time, so installing it later still works.
    static func gitExecutable(searching path: String) -> URL? {
        GitExecutableCache.shared.lock.lock()
        defer { GitExecutableCache.shared.lock.unlock() }
        if let found = GitExecutableCache.shared.paths[path] { return found }
        for directory in path.split(separator: ":") where !directory.isEmpty {
            let candidate = URL(fileURLWithPath: String(directory)).appendingPathComponent("git")
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                GitExecutableCache.shared.paths[path] = candidate
                return candidate
            }
        }
        return nil
    }
}

final class GitExecutableCache: @unchecked Sendable {
    static let shared = GitExecutableCache()
    let lock = NSLock()
    var paths: [String: URL] = [:]
}

extension GitClient {
    /// How many Git processes a refresh starts at once. Measured on a 14-core Mac, five to seven
    /// finished a refresh about 10% sooner than nine, and three took a third longer; this
    /// scales the limit to the machine.
    static let concurrentCommandLimit = min(6, max(2, ProcessInfo.processInfo.activeProcessorCount / 2))
}

