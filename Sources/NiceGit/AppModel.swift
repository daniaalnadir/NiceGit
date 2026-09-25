import AppKit
import Combine
import Foundation
import NiceGitCore

@MainActor
final class AppModel: ObservableObject {
    @Published var snapshot: RepositorySnapshot?
    @Published var recentRepositories: [RepositoryBookmark]
    @Published private(set) var openRepositories: [RepositoryBookmark] = []
    @Published var errorMessage: String?
    @Published var noticeMessage: String?
    @Published var isLoading = false
    @Published var showingTerminal = false
    @Published private(set) var activeTerminal: TerminalSession?
    private var terminalSessions: [String: TerminalSession] = [:]

    func toggleTerminal() {
        guard snapshot != nil, !isLoading else { return }
        if showingTerminal {
            showingTerminal = false
            refresh()
        } else {
            openTerminal()
        }
    }

    func openTerminal(restart: Bool = false) {
        guard let path = snapshot?.rootPath else { return }
        if terminalSessions[path] == nil || (restart && terminalSessions[path]?.ended == true) {
            terminalSessions[path] = TerminalSession(path: path)
        }
        activeTerminal = terminalSessions[path]
        showingTerminal = true
    }
    struct CommitHistoryStep {
        let path: String
        let branch: String
        let before: String
        let after: String
        var undone = false
    }
    @Published private(set) var commitHistoryStep: CommitHistoryStep?

    var canUndoCommit: Bool { canMoveCommitHistory(undone: false) }
    var canRedoCommit: Bool { canMoveCommitHistory(undone: true) }

    private func canMoveCommitHistory(undone: Bool) -> Bool {
        guard !isLoading, !fileReviewHasEdits, let step = commitHistoryStep, let snapshot else { return false }
        return step.undone == undone && snapshot.rootPath == step.path && snapshot.currentBranch == step.branch
            && snapshot.headHash == (undone ? step.before : step.after) && snapshot.operation == nil
    }

    func moveCommitHistory(redo: Bool) {
        guard canMoveCommitHistory(undone: redo), let step = commitHistoryStep else { return }
        runRepositoryAction({ git, url in
            try git.reset(to: redo ? step.after : step.before, mode: .soft,
                          expectedHead: redo ? step.before : step.after, expectedBranch: step.branch, in: url)
        }, onSuccess: { self.commitHistoryStep?.undone = !redo })
    }
    @Published var diffSelection: DiffSelection?
    @Published var fileReviewSelection: DiffSelection?
    @Published var fileReviewHasEdits = false

    func confirmDiscardFileEdits() -> Bool {
        guard fileReviewHasEdits else { return true }
        let alert = NSAlert()
        alert.messageText = "Discard unsaved file edits?"
        alert.informativeText = "Your changes in the code editor have not been saved to disk."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Discard edits")
        guard alert.runModal() == .alertSecondButtonReturn else { return false }
        fileReviewHasEdits = false
        return true
    }

    private func requireSavedFileEdits(before action: String) -> Bool {
        guard !fileReviewHasEdits else {
            errorMessage = "Save or discard your unsaved file edits before \(action)."
            return false
        }
        return true
    }

    func closeFileReview() {
        guard confirmDiscardFileEdits() else { return }
        fileReviewSelection = nil
    }
    @Published var showingClone = false
    @Published var showingStashes = false
    @Published var showingPublish = false
    @Published var showingRepositorySettings = false
    @Published var taggingCommit: GitCommit?
    @Published var editingCommitMessage: GitCommit?
    @Published private var commitDrafts: [String: String] = [:]

    func commitDraft(for path: String) -> String { commitDrafts[path] ?? "" }

    func setCommitDraft(_ message: String, for path: String) {
        commitDrafts[path] = message.isEmpty ? nil : message
        defaults.set(commitDrafts, forKey: draftKey)
    }
    private var activationRefreshPending = false
    private let snapshotLoader: @Sendable (GitClient, URL, Int) throws -> RepositorySnapshot

    func createTag(name: String, target: String, message: String? = nil) {
        runRepositoryAction({ git, url in try git.createTag(name: name, target: target, message: message, in: url) }, onSuccess: { self.taggingCommit = nil })
    }

    func deleteTag(name: String, expectedTip: String) {
        runRepositoryAction { git, url in try git.deleteTag(name: name, expectedTip: expectedTip, in: url) }
    }

    func amendMessage(_ message: String, for commit: GitCommit) {
        runRepositoryAction({ git, url in try git.amendMessage(message, expectedHead: commit.hash, in: url) }, onSuccess: { self.editingCommitMessage = nil })
    }

    func inspectTag(_ name: String) {
        guard let repositoryURL else { return }
        diffSelection = DiffSelection(title: name, repositoryURL: repositoryURL, commitHash: "refs/tags/" + name)
    }
    private var historyLimit = 200

    func loadOlderCommits() {
        guard !isLoading, snapshot?.hasMoreCommits == true else { return }
        historyLimit += 200
        refresh()
    }

    func initializeRepository() {
        guard !isLoading else { return }
        let panel = NSOpenPanel()
        panel.title = "Create repository"
        panel.message = "Choose a folder for the new repository"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        guard confirmDiscardFileEdits() else { return }
        fileReviewSelection = nil
        perform(at: url, action: { git, url in try git.initialize(at: url) }, onSuccess: { self.showingRepositorySettings = true })
    }

    func setIdentity(name: String, email: String, onSuccess: @escaping () -> Void) {
        runRepositoryAction({ git, url in try git.setIdentity(name: name, email: email, in: url) }, onSuccess: onSuccess)
    }

    func addRemote(name: String, address: String, onSuccess: @escaping () -> Void) {
        runRepositoryAction({ git, url in try git.addRemote(name: name, address: address, in: url) }, onSuccess: onSuccess)
    }

    func publish(remote: String) {
        let branch = snapshot?.currentBranch
        let head = snapshot?.headHash
        runRepositoryAction({ git, url in
            try git.publish(remote: remote, expectedBranch: branch, expectedHead: head, in: url)
        }, onSuccess: { self.showingPublish = false })
    }

    func start(_ operation: GitOperation, target: String, mainline: Int? = nil, expectedHead: String? = nil, expectedBranch: String? = nil, expectedSourceBranch: GitBranch? = nil) {
        guard requireSavedFileEdits(before: "starting a Git operation") else { return }
        let head = expectedHead ?? snapshot?.headHash
        let branch = expectedBranch ?? snapshot?.currentBranch
        runRepositoryAction { git, url in
            try git.start(operation, target: target, mainline: mainline, expectedHead: head, expectedBranch: branch, expectedSourceBranch: expectedSourceBranch, in: url)
        }
    }

    func reset(to target: String, mode: GitResetMode, expectedHead: String, expectedBranch: String) {
        guard requireSavedFileEdits(before: "resetting") else { return }
        let reviewID = fileReviewSelection?.id
        runRepositoryAction({ git, url in
            try git.reset(to: target, mode: mode, expectedHead: expectedHead, expectedBranch: expectedBranch, in: url)
        }, onSuccess: {
            if self.fileReviewSelection?.id == reviewID { self.fileReviewSelection = nil }
        })
    }

    func continueOperation() {
        guard requireSavedFileEdits(before: "continuing the Git operation") else { return }
        guard let operation = snapshot?.operation else { return }
        runRepositoryAction { git, url in try git.continueOperation(operation, in: url) }
    }

    func abortOperation() {
        guard requireSavedFileEdits(before: "aborting the Git operation") else { return }
        guard let operation = snapshot?.operation else { return }
        runRepositoryAction { git, url in try git.abortOperation(operation, in: url) }
    }

    func saveStash(message: String, includeUntracked: Bool, onSuccess: @escaping () -> Void) {
        guard requireSavedFileEdits(before: "stashing") else { return }
        runRepositoryAction({ git, url in
            try git.saveStash(message: message, includeUntracked: includeUntracked, in: url)
        }, onSuccess: onSuccess)
    }

    func applyStash(_ stash: GitStash) {
        guard requireSavedFileEdits(before: "applying a stash") else { return }
        runRepositoryAction { git, url in try git.applyStash(stash, in: url) }
    }

    func popStash(_ stash: GitStash) {
        guard requireSavedFileEdits(before: "popping a stash") else { return }
        runRepositoryAction { git, url in try git.popStash(stash, in: url) }
    }

    func dropStash(_ stash: GitStash) {
        runRepositoryAction { git, url in try git.dropStash(stash, in: url) }
    }

    func clone(source: String) {
        guard !isLoading else { return }
        let panel = NSSavePanel()
        panel.title = "Clone repository"
        panel.prompt = "Clone"
        panel.nameFieldStringValue = "repository"
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        guard confirmDiscardFileEdits() else { return }
        fileReviewSelection = nil
        showingClone = false
        perform(at: destination) { git, url in
            try git.clone(source: source, to: url)
        }
    }

    func createWorktree(for branch: GitBranch) {
        guard !isLoading, !branch.isRemote else { return }
        let panel = NSSavePanel()
        panel.title = "Create worktree"
        panel.prompt = "Create worktree"
        panel.nameFieldStringValue = branch.name.replacingOccurrences(of: "/", with: "-")
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        runRepositoryAction { git, url in
            try git.createWorktree(branch: branch.name, at: destination, in: url)
        }
    }

    func inspect(_ entry: GitStatusEntry, staged: Bool) {
        guard let repositoryURL else { return }
        guard confirmDiscardFileEdits() else { return }
        let selection = DiffSelection(title: entry.path, repositoryURL: repositoryURL, path: entry.path, staged: staged, untracked: entry.kind == .untracked, conflicted: entry.kind == .conflicted, originalPath: entry.originalPath)
        if selection.conflicted { diffSelection = selection }
        else { fileReviewSelection = selection }
    }

    func importPatch() {
        guard !isLoading, let url = repositoryURL else { return }
        guard requireSavedFileEdits(before: "applying a patch") else { return }
        let panel = NSOpenPanel()
        panel.title = "Apply patch"
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let source = panel.url else { return }
        let alert = NSAlert()
        alert.messageText = "Apply \(source.lastPathComponent)?"
        alert.informativeText = "Applies file changes to \(url.path). Changes remain unstaged and no commit is created. Git checks the patch before applying it."
        alert.addButton(withTitle: "Apply patch")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        perform(at: url) { git, repository in
            try git.applyPatch(Data(contentsOf: source), in: repository)
        }
    }

    func exportPatch(_ commit: GitCommit) {
        guard !isLoading, let url = repositoryURL else { return }
        let panel = NSSavePanel()
        panel.title = "Export commit patch"
        panel.nameFieldStringValue = "\(commit.shortHash).patch"
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        perform(at: url) { git, repository in
            let patch = try git.exportCommitPatch(hash: commit.hash, in: repository)
            try patch.write(to: destination, atomically: true, encoding: .utf8)
        }
    }

    func inspect(_ commit: GitCommit) {
        guard let repositoryURL else { return }
        diffSelection = DiffSelection(title: commit.subject, repositoryURL: repositoryURL, commitHash: commit.hash)
    }

    private let recentKey = "NiceGit.recentRepositories"
    private let draftKey = "NiceGit.commitDrafts"
    private let tabsKey = "NiceGit.openRepositories"
    private let activeTabKey = "NiceGit.activeRepository"
    private var didRestoreSession = false
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard, snapshotLoader: @escaping @Sendable (GitClient, URL, Int) throws -> RepositorySnapshot = { git, url, limit in
        try git.loadSnapshot(at: url, historyLimit: limit)
    }) {
        self.snapshotLoader = snapshotLoader
        self.defaults = defaults
        commitDrafts = defaults.dictionary(forKey: draftKey) as? [String: String] ?? [:]
        if let data = defaults.data(forKey: tabsKey),
           let saved = try? JSONDecoder().decode([RepositoryBookmark].self, from: data) {
            var seen = Set<String>()
            openRepositories = saved.filter { $0.path.hasPrefix("/") && seen.insert($0.path).inserted }
        }
        if let data = defaults.data(forKey: recentKey),
           let bookmarks = try? JSONDecoder().decode([RepositoryBookmark].self, from: data) {
            recentRepositories = bookmarks
        } else {
            recentRepositories = []
        }
    }

    var repositoryURL: URL? {
        snapshot.map { URL(fileURLWithPath: $0.rootPath) }
    }

    func restoreSession() {
        guard !didRestoreSession else { return }
        didRestoreSession = true
        guard snapshot == nil, !isLoading else { return }
        let active = defaults.string(forKey: activeTabKey)
        if let tab = openRepositories.first(where: { $0.path == active }) ?? openRepositories.first {
            loadRepository(at: URL(fileURLWithPath: tab.path))
        }
    }

    private func saveOpenTabs() {
        if let data = try? JSONEncoder().encode(openRepositories) { defaults.set(data, forKey: tabsKey) }
    }

    func closeRepository(path: String) {
        guard !isLoading, let index = openRepositories.firstIndex(where: { $0.path == path }) else { return }
        if snapshot?.rootPath == path {
            guard confirmDiscardFileEdits() else { return }
            fileReviewSelection = nil
        }
        openRepositories.remove(at: index)
        saveOpenTabs()
        if defaults.string(forKey: activeTabKey) == path { defaults.removeObject(forKey: activeTabKey) }
        guard snapshot?.rootPath == path else { return }
        snapshot = nil
        errorMessage = nil
        if !openRepositories.isEmpty {
            let next = openRepositories[min(index, openRepositories.count - 1)]
            loadRepository(at: URL(fileURLWithPath: next.path))
        }
    }

    func openRepository() {
        guard !isLoading else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.message = "Choose a Git repository"

        guard panel.runModal() == .OK, let url = panel.url else {
            return
        }

        loadRepository(at: url)
    }

    func loadRepository(at url: URL) {
        guard !isLoading else { return }
        if url.standardizedFileURL.path != repositoryURL?.standardizedFileURL.path {
            guard confirmDiscardFileEdits() else { return }
            historyLimit = 200
            fileReviewSelection = nil
        }
        perform(at: url) { _, _ in }
    }

    func refresh() {
        guard let repositoryURL else {
            return
        }

        loadRepository(at: repositoryURL)
    }

    func refreshOnActivation() {
        guard !isLoading, errorMessage == nil else { return }
        activationRefreshPending = true
        refreshAfterReview()
    }

    func refreshAfterReview() {
        guard activationRefreshPending, !isLoading, errorMessage == nil, diffSelection == nil,
              !showingClone, !showingStashes, !showingPublish,
              !showingRepositorySettings, taggingCommit == nil, editingCommitMessage == nil else { return }
        activationRefreshPending = false
        refresh()
    }

    func stage(_ entry: GitStatusEntry) {
        runWorkingTreeAction { git, url in
            try git.stage(path: entry.path, in: url)
        }
    }

    func stageAll() {
        runWorkingTreeAction { git, url in
            try git.stageAll(in: url)
        }
    }

    func unstage(_ entry: GitStatusEntry) {
        runWorkingTreeAction { git, url in
            try git.unstage(path: entry.path, originalPath: entry.originalPath, in: url)
        }
    }

    func unstageAll() {
        runWorkingTreeAction { git, url in
            try git.unstageAll(in: url)
        }
    }

    func discard(_ entry: GitStatusEntry) {
        if fileReviewSelection?.path == entry.path {
            guard confirmDiscardFileEdits() else { return }
            fileReviewSelection = nil
        }
        runWorkingTreeAction { git, url in
            try git.discard(entry, in: url)
        }
    }

    func commit(message: String, onSuccess: @escaping () -> Void) {
        guard !isLoading, let url = repositoryURL else { return }
        guard requireSavedFileEdits(before: "committing") else { return }
        let reviewID = fileReviewSelection?.id
        let previous = snapshot
        perform(at: url, action: { git, url in
            try git.commit(message: message, in: url)
        }, onActionSuccess: {
            if self.fileReviewSelection?.id == reviewID && !self.fileReviewHasEdits {
                self.fileReviewSelection = nil
            }
            onSuccess()
        }, onSuccess: {
            self.commitHistoryStep = nil
            if let previous, previous.operation == nil, let before = previous.headHash,
               let updated = self.snapshot, updated.currentBranch == previous.currentBranch,
               let after = updated.headHash, after != before,
               updated.commits.first(where: { $0.hash == after })?.parents.first == before {
                self.commitHistoryStep = CommitHistoryStep(path: updated.rootPath, branch: updated.currentBranch, before: before, after: after)
            }
        })
    }

    func checkout(branch: GitBranch) {
        guard !isLoading, !branch.isCurrent else { return }
        if branch.isRemote, snapshot?.branches.contains(where: {
            $0.isCurrent && $0.upstream == "refs/" + branch.name
        }) == true { return }
        let discardEditorEdits = fileReviewHasEdits
        guard confirmDiscardFileEdits() else { return }
        if discardEditorEdits { fileReviewSelection = nil }
        noticeMessage = nil
        let outcome = BranchSwitchOutcome()
        runRepositoryAction({ git, url in
            let savedChanges: Bool
            if branch.isRemote { savedChanges = try git.checkoutRemote(branch: branch.name, expectedTip: branch.tip, in: url) }
            else { savedChanges = try git.checkout(branch: branch.name, expectedTip: branch.tip, in: url) }
            outcome.record(savedChanges)
        }, onSuccess: {
            self.fileReviewSelection = nil
        }, onRefreshed: {
            if outcome.savedChanges {
                self.noticeMessage = "Your uncommitted changes were saved in Stashes before switching branches. Apply the NiceGit stash to restore them."
            }
        })
    }

    func renameBranch(_ branch: GitBranch, to name: String) {
        runRepositoryAction { git, url in try git.renameBranch(branch.name, to: name, expectedTip: branch.tip, in: url) }
    }

    func deleteBranch(_ branch: GitBranch) {
        runRepositoryAction { git, url in try git.deleteBranch(branch.name, expectedTip: branch.tip, in: url) }
    }

    func createBranch(named name: String, onSuccess: @escaping () -> Void) {
        runRepositoryAction({ git, url in
            try git.createBranch(named: name, in: url)
        }, onSuccess: onSuccess)
    }

    func createBranch(named name: String, from branch: GitBranch, onSuccess: @escaping () -> Void) {
        runRepositoryAction({ git, url in
            try git.createBranch(named: name, startingAt: branch.tip, expectedSourceBranch: branch, in: url)
        }, onSuccess: onSuccess)
    }

    func fetch() {
        runRepositoryAction { git, url in
            try git.fetch(in: url)
        }
    }

    func setUpstream(for branch: GitBranch, to remoteBranch: GitBranch?) {
        guard !branch.isRemote else { return }
        let remoteName = remoteBranch.map { String($0.name.dropFirst("remotes/".count)) }
        runRepositoryAction { git, url in
            try git.setUpstream(branch: branch.name, remoteBranch: remoteName, expectedTip: branch.tip, in: url)
        }
    }

    func pull() {
        guard requireSavedFileEdits(before: "pulling") else { return }
        runRepositoryAction { git, url in
            try git.pull(in: url)
        }
    }

    func push() {
        guard let snapshot, snapshot.upstream != nil else {
            showingPublish = true
            return
        }
        runRepositoryAction { git, url in
            try git.push(expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: url)
        }
    }

    func push(_ branch: GitBranch, to remote: String) {
        guard !branch.isRemote else { return }
        runRepositoryAction { git, url in try git.pushBranch(branch.name, to: remote, expectedTip: branch.tip, in: url) }
    }

    func copyCommitLink(hash: String, remote: String) {
        guard let directory = repositoryURL else { return }
        Task {
            do {
                let link = try await Task.detached {
                    let address = try GitClient().remoteAddress(name: remote, in: directory)
                    return try GitHubRepository(remoteAddress: address).commitURL(hash: hash)
                }.value
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(link.absoluteString, forType: .string)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func refreshWorkingTree() {
        runWorkingTreeAction { _, _ in }
    }

    private func runWorkingTreeAction(_ action: @escaping @Sendable (GitClient, URL) throws -> Void) {
        guard let repositoryURL else { return }
        perform(at: repositoryURL, action: action, statusOnly: true)
    }

    private func runRepositoryAction(_ action: @escaping @Sendable (GitClient, URL) throws -> Void, onSuccess: @escaping () -> Void = {}, onRefreshed: @escaping () -> Void = {}) {
        guard let repositoryURL else { return }
        perform(at: repositoryURL, action: action, onActionSuccess: onSuccess, onSuccess: onRefreshed)
    }

    private func perform(at url: URL, action: @escaping @Sendable (GitClient, URL) throws -> Void, statusOnly: Bool = false, onActionSuccess: (() -> Void)? = nil, onSuccess: @escaping () -> Void = {}) {
        guard !isLoading else { return }
        isLoading = true
        errorMessage = nil
        let limit = historyLimit
        let loadSnapshot = snapshotLoader
        let previous = snapshot
        Task {
            defer { isLoading = false }
            var actionCompleted = false
            do {
                try await Task.detached {
                    try action(GitClient(), url)
                }.value
                // A successful mutation stays successful even if refreshing its result fails.
                actionCompleted = true
                onActionSuccess?()
                let updated = try await Task.detached {
                    let git = GitClient()
                    if statusOnly, var cached = previous, cached.rootPath == url.path {
                        cached.status = try git.loadStatus(in: url)
                        cached.lastUpdated = Date()
                        return cached
                    }
                    return try loadSnapshot(git, url, limit)
                }.value
                snapshot = updated
                if !statusOnly { rememberRepository(path: updated.rootPath) }
                onSuccess()
            } catch {
                errorMessage = actionCompleted && (onActionSuccess != nil || statusOnly)
                    ? "The Git action completed, but the repository could not be refreshed. Refresh before repeating the action.\n\n\(error.localizedDescription)"
                    : error.localizedDescription
                // Failed operations such as stash apply may still change files.
                if let refreshed = try? await Task.detached(operation: { try loadSnapshot(GitClient(), url, limit) }).value {
                    snapshot = refreshed
                    rememberRepository(path: refreshed.rootPath)
                }
            }
        }
    }

    private func rememberRepository(path: String) {
        let bookmark = RepositoryBookmark(path: path)
        if !openRepositories.contains(where: { $0.path == path }) { openRepositories.append(bookmark) }
        saveOpenTabs()
        defaults.set(path, forKey: activeTabKey)
        recentRepositories.removeAll { $0.path == path }
        recentRepositories.insert(bookmark, at: 0)
        recentRepositories = Array(recentRepositories.prefix(8))

        if let data = try? JSONEncoder().encode(recentRepositories) {
            defaults.set(data, forKey: recentKey)
        }
    }
}

private final class BranchSwitchOutcome: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false

    func record(_ savedChanges: Bool) {
        lock.lock()
        value = savedChanges
        lock.unlock()
    }

    var savedChanges: Bool {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

struct RepositoryBookmark: Codable, Identifiable, Equatable {
    var path: String

    var id: String {
        path
    }

    var name: String {
        URL(fileURLWithPath: path).lastPathComponent
    }
}
