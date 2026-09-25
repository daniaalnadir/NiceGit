import Foundation
@testable import NiceGit
import NiceGitCore
import Testing

// Synchronous Git fixture setup must not starve another test's main-actor callbacks.
@Suite(.serialized)
struct AppModelTests {

@Test @MainActor func stagingRefreshesStatusWithoutReloadingHistory() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try "example\n".write(to: root.appendingPathComponent("example.txt"), atomically: true, encoding: .utf8)
    let clock = ContinuousClock()
    let fullStart = clock.now
    let initial = try git.loadSnapshot(at: root)
    let fullDuration = fullStart.duration(to: clock.now)
    let statusStart = clock.now
    #expect(try git.loadStatus(in: root) == initial.status)
    print("Refresh timing: full snapshot \(fullDuration), status only \(statusStart.duration(to: clock.now))")
    let model = AppModel(defaults: defaults, snapshotLoader: { _, _, _ in
        throw GitClientError.commandFailed(command: "test", message: "Unexpected full history refresh")
    })
    model.snapshot = initial
    for stage in [true, false] {
        let start = clock.now
        if stage { model.stageAll() } else { model.unstageAll() }
        let deadline = clock.now.advanced(by: .seconds(10))
        while model.isLoading && clock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
        #expect(!model.isLoading)
        #expect(model.errorMessage == nil)
        #expect(model.snapshot?.stagedCount == (stage ? 1 : 0))
        #expect(model.snapshot?.commits == initial.commits)
        #expect(model.snapshot?.branches == initial.branches)
        #expect(try String(contentsOf: root.appendingPathComponent("example.txt"), encoding: .utf8) == "example\n")
        print("\(stage ? "Stage" : "Unstage") including UI status update: \(start.duration(to: clock.now))")
    }
}

@Test @MainActor func stagingAfterExternalCheckoutReloadsBranchAndHistory() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let model = AppModel(defaults: defaults)
    model.snapshot = try git.loadSnapshot(at: root)
    let base = try #require(model.snapshot?.headHash)

    try git.createBranch(named: "external", in: root)
    try "changed\n".write(to: file, atomically: true, encoding: .utf8)
    model.stageAll()
    let firstDeadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < firstDeadline { try await Task.sleep(for: .milliseconds(20)) }
    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.currentBranch == "external")
    #expect(model.snapshot?.headHash == base)
    #expect(model.snapshot?.stagedCount == 1)

    try git.commit(message: "External commit", in: root)
    let newHead = try #require(git.loadSnapshot(at: root).headHash)
    try "more changes\n".write(to: file, atomically: true, encoding: .utf8)
    model.stageAll()
    let secondDeadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < secondDeadline { try await Task.sleep(for: .milliseconds(20)) }
    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.headHash == newHead)
    #expect(model.snapshot?.commits.first?.hash == newHead)
}

@Test @MainActor func terminalToggleRefreshesChangesAndRespectsBusyState() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults)
    model.toggleTerminal()
    #expect(!model.showingTerminal)
    #expect(model.activeTerminal == nil)

    let git = GitClient()
    try git.initialize(at: root)
    model.snapshot = try git.loadSnapshot(at: root)
    model.isLoading = true
    model.toggleTerminal()
    #expect(!model.showingTerminal)
    model.isLoading = false
    model.toggleTerminal()
    let session = try #require(model.activeTerminal)
    #expect(model.showingTerminal)

    model.isLoading = true
    model.toggleTerminal()
    #expect(model.showingTerminal)
    #expect(model.activeTerminal === session)
    model.isLoading = false
    try "external change\n".write(to: root.appendingPathComponent("external.txt"), atomically: true, encoding: .utf8)
    model.toggleTerminal()
    #expect(!model.showingTerminal)
    #expect(model.isLoading)
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    try #require(!model.isLoading)
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.status.contains { $0.path == "external.txt" } == true)
    model.toggleTerminal()
    #expect(model.showingTerminal)
    #expect(model.activeTerminal === session)
}

@Test @MainActor func terminalSessionsRemainSeparateAndPreserveLiveShells() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults)
    model.openTerminal()
    #expect(model.activeTerminal == nil)
    #expect(!model.showingTerminal)
    let git = GitClient()
    let first = root.appendingPathComponent("first")
    let second = root.appendingPathComponent("second")
    for path in [first, second] {
        try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
        try git.initialize(at: path)
    }
    model.snapshot = try git.loadSnapshot(at: first)
    model.openTerminal()
    let a = try #require(model.activeTerminal)
    #expect(a.path == model.snapshot?.rootPath)
    model.showingTerminal = false
    model.openTerminal(restart: true)
    #expect(model.activeTerminal === a)
    #expect(model.showingTerminal)
    model.snapshot = try git.loadSnapshot(at: second)
    model.openTerminal()
    let b = try #require(model.activeTerminal)
    #expect(b !== a)
    #expect(b.path == model.snapshot?.rootPath)
    model.snapshot = try git.loadSnapshot(at: first)
    model.openTerminal()
    #expect(model.activeTerminal === a)
    a.processTerminated(source: a.view, exitCode: 0)
    let deadline = ContinuousClock.now.advanced(by: .seconds(2))
    while !a.ended && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(a.ended)
    #expect(a.status == "Shell exited (0)")
    model.openTerminal(restart: true)
    let replacement = try #require(model.activeTerminal)
    #expect(replacement !== a)
    #expect(replacement.path == a.path)
    #expect(!replacement.ended)
    model.snapshot = try git.loadSnapshot(at: second)
    model.openTerminal()
    #expect(model.activeTerminal === b)
}

@Test @MainActor func commitUndoRedoPreservesWorkingFiles() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let base = try #require(git.loadSnapshot(at: root).headHash)
    let model = AppModel(defaults: defaults)
    model.snapshot = try git.loadSnapshot(at: root)
    @MainActor func wait() async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(15))
        while model.isLoading && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        #expect(!model.isLoading)
    }
    try "committed\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    model.commit(message: "App commit") {}
    try await wait()
    #expect(model.errorMessage == nil)
    #expect(model.canUndoCommit)
    let committed = try #require(model.snapshot?.headHash)
    try "later edits\n".write(to: file, atomically: true, encoding: .utf8)
    model.moveCommitHistory(redo: false)
    try await wait()
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.headHash == base)
    #expect(model.canRedoCommit)
    #expect(try git.diff(path: "file.txt", staged: true, in: root).contains("+committed"))
    #expect(try String(contentsOf: file, encoding: .utf8) == "later edits\n")
    model.moveCommitHistory(redo: true)
    try await wait()
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.headHash == committed)
    #expect(try String(contentsOf: file, encoding: .utf8) == "later edits\n")
    try git.stageAll(in: root)
    try git.commit(message: "External commit", in: root)
    let external = try git.loadSnapshot(at: root).headHash
    model.moveCommitHistory(redo: false)
    try await wait()
    #expect(model.errorMessage != nil)
    #expect(try git.loadSnapshot(at: root).headHash == external)
    #expect(!model.canUndoCommit)
}

@Test @MainActor func hardResetDoesNotRunWhileFileEditorHasUnsavedChanges() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let base = try #require(git.loadSnapshot(at: root).headHash)
    try "current\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Current", in: root)
    let model = AppModel(defaults: defaults)
    model.snapshot = try git.loadSnapshot(at: root)
    let current = try #require(model.snapshot?.headHash)
    model.fileReviewSelection = DiffSelection(title: "file.txt", repositoryURL: root, path: "file.txt")
    model.fileReviewHasEdits = true

    model.reset(to: base, mode: .hard, expectedHead: current, expectedBranch: "main")
    let deadline = ContinuousClock.now.advanced(by: .seconds(10))
    while model.isLoading && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }

    #expect(!model.isLoading)
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(try git.loadSnapshot(at: root).headHash == current)
    #expect(try String(contentsOf: file, encoding: .utf8) == "current\n")
    #expect(model.fileReviewHasEdits)

    model.errorMessage = nil
    model.saveStash(message: "Editor is dirty", includeUntracked: true) {}
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(!model.isLoading)
    model.errorMessage = nil
    model.pull()
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(!model.isLoading)
    model.errorMessage = nil
    model.start(.merge, target: base)
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(!model.isLoading)
    model.errorMessage = nil
    model.createBranch(named: "unsaved-branch", expectedBranch: "main", expectedHead: current) {}
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(!model.isLoading)
    #expect(try git.loadSnapshot(at: root).branches.allSatisfy { $0.name != "unsaved-branch" })
    #expect(try git.loadSnapshot(at: root).headHash == current)

    model.fileReviewHasEdits = false
    model.errorMessage = nil
    model.reset(to: base, mode: .hard, expectedHead: current, expectedBranch: "main")
    let successDeadline = ContinuousClock.now.advanced(by: .seconds(10))
    while model.isLoading && ContinuousClock.now < successDeadline { try await Task.sleep(for: .milliseconds(20)) }
    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
    #expect(model.fileReviewSelection == nil)
    #expect(try String(contentsOf: file, encoding: .utf8) == "base\n")
}

@Test @MainActor func openTabsDeduplicateAndCloseWithoutLosingDrafts() async throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let first = base.appendingPathComponent("first")
    let second = base.appendingPathComponent("second")
    for url in [first, second] {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try GitClient().initialize(at: url)
    }
    defer { try? FileManager.default.removeItem(at: base) }
    let a = try GitClient().loadSnapshot(at: first)
    let b = try GitClient().loadSnapshot(at: second)
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults, snapshotLoader: { _, url, _ in url.lastPathComponent == "first" ? a : b })
    @MainActor func waitForLoad() async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        while model.isLoading && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        #expect(!model.isLoading)
        #expect(model.errorMessage == nil)
    }
    for url in [first, second, first] {
        model.loadRepository(at: url)
        try await waitForLoad()
    }
    #expect(model.openRepositories.map(\.path) == [a.rootPath, b.rootPath])
    let restored = AppModel(defaults: defaults, snapshotLoader: { _, url, _ in url.lastPathComponent == "first" ? a : b })
    #expect(restored.openRepositories.map(\.path) == [a.rootPath, b.rootPath])
    restored.restoreSession()
    let deadline = ContinuousClock.now.advanced(by: .seconds(10))
    while restored.isLoading && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
    #expect(!restored.isLoading)
    #expect(restored.snapshot?.rootPath == a.rootPath)
    restored.restoreSession()
    #expect(!restored.isLoading)
    model.setCommitDraft("Keep this", for: a.rootPath)
    model.closeRepository(path: b.rootPath)
    #expect(model.snapshot?.rootPath == a.rootPath)
    model.loadRepository(at: second)
    try await waitForLoad()
    model.closeRepository(path: b.rootPath)
    try await waitForLoad()
    #expect(model.snapshot?.rootPath == a.rootPath)
    model.closeRepository(path: a.rootPath)
    #expect(model.snapshot == nil)
    #expect(model.openRepositories.isEmpty)
    #expect(model.commitDraft(for: a.rootPath) == "Keep this")
    #expect(model.recentRepositories.count == 2)
    let empty = AppModel(defaults: defaults)
    empty.restoreSession()
    #expect(empty.openRepositories.isEmpty)
    #expect(empty.snapshot == nil)
    #expect(!empty.isLoading)
}

@Test @MainActor func successfulCommitClearsDraftWhenRefreshFails() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "content\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults, snapshotLoader: { _, _, _ in throw RefreshFailure.injected })
    model.snapshot = try git.loadSnapshot(at: root)
    var draft = "Successful commit"
    model.fileReviewSelection = DiffSelection(title: "file.txt", repositoryURL: root, path: "file.txt", staged: true)
    var successes = 0

    model.commit(message: draft) { draft = ""; successes += 1 }
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(draft.isEmpty)
    #expect(model.fileReviewSelection == nil)
    #expect(successes == 1)
    #expect(model.errorMessage?.contains("The Git action completed") == true)
    let actual = try git.loadSnapshot(at: root)
    #expect(actual.commits.count == 1)
    #expect(actual.commits.first?.subject == "Successful commit")
    #expect(actual.status.isEmpty)
}

private enum RefreshFailure: Error { case injected }

@Test @MainActor func commitDraftsRemainSeparateForEachCheckout() throws {
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults)
    model.setCommitDraft("Main checkout draft", for: "/repo/main")
    model.setCommitDraft("Linked checkout draft", for: "/repo/linked")
    #expect(model.commitDraft(for: "/repo/main") == "Main checkout draft")
    #expect(model.commitDraft(for: "/repo/linked") == "Linked checkout draft")
    model.setCommitDraft("", for: "/repo/main")
    #expect(model.commitDraft(for: "/repo/main").isEmpty)
    #expect(model.commitDraft(for: "/repo/linked") == "Linked checkout draft")
    #expect(model.commitDraft(for: "/unknown").isEmpty)
    let reopened = AppModel(defaults: defaults)
    #expect(reopened.commitDraft(for: "/repo/main").isEmpty)
    #expect(reopened.commitDraft(for: "/repo/linked") == "Linked checkout draft")
    reopened.setCommitDraft("", for: "/repo/linked")
    #expect(AppModel(defaults: defaults).commitDraft(for: "/repo/linked").isEmpty)
}

@Test @MainActor func activationRefreshWaitsForReviewDismissal() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults, snapshotLoader: { _, _, _ in throw RefreshFailure.injected })
    model.snapshot = try git.loadSnapshot(at: root)
    model.diffSelection = DiffSelection(title: "Review", repositoryURL: root)

    model.refreshOnActivation()
    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
    model.diffSelection = nil
    model.refreshAfterReview()
    #expect(model.isLoading)
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(!model.isLoading)
    #expect(model.errorMessage != nil)
    model.errorMessage = nil
    model.refreshAfterReview()
    #expect(!model.isLoading)
}

@Test @MainActor func activationDoesNotQueueRefreshDuringOperation() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults, snapshotLoader: { _, _, _ in throw RefreshFailure.injected })
    model.snapshot = try git.loadSnapshot(at: root)
    model.isLoading = true
    model.refreshOnActivation()
    #expect(model.isLoading)
    model.isLoading = false
    model.refreshAfterReview()
    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
}

@Test @MainActor func failedCommitKeepsDraftAndDoesNotReportSuccess() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = AppModel(defaults: defaults, snapshotLoader: { _, _, _ in throw RefreshFailure.injected })
    model.snapshot = try git.loadSnapshot(at: root)
    var draft = "Keep this draft"
    let review = DiffSelection(title: "file.txt", repositoryURL: root, path: "file.txt", staged: true)
    model.fileReviewSelection = review
    var successes = 0

    model.fileReviewHasEdits = true
    model.commit(message: draft) { successes += 1 }
    #expect(!model.isLoading)
    #expect(model.fileReviewSelection?.id == review.id)
    #expect(model.fileReviewHasEdits)
    #expect(model.errorMessage?.contains("unsaved file edits") == true)
    #expect(successes == 0)
    model.fileReviewHasEdits = false

    model.commit(message: draft) { draft = ""; successes += 1 }
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(draft == "Keep this draft")
    #expect(model.fileReviewSelection?.id == review.id)
    #expect(successes == 0)
    #expect(model.errorMessage != nil)
    #expect(model.errorMessage?.contains("The Git action completed") == false)
    #expect(try git.loadSnapshot(at: root).commits.isEmpty)
}

@Test @MainActor func branchSwitchReportsActualStashWhenSnapshotIsStale() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", startingAt: "HEAD", in: root)
    let model = AppModel(defaults: defaults)
    model.snapshot = try git.loadSnapshot(at: root)
    #expect(model.snapshot?.status.isEmpty == true)
    let feature = try #require(model.snapshot?.branches.first { $0.name == "feature" })

    try "external edit\n".write(to: file, atomically: true, encoding: .utf8)
    model.checkout(branch: feature)
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(model.errorMessage == nil)
    #expect(model.snapshot?.currentBranch == "feature")
    #expect(model.snapshot?.stashes.first?.message.contains("main before switching to feature") == true)
    #expect(model.noticeMessage?.contains("saved in Stashes") == true)
}

@Test @MainActor func busyRepositoryLoadKeepsCurrentReview() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    let model = AppModel(defaults: defaults)
    let original = try git.loadSnapshot(at: root)
    model.snapshot = original
    let review = DiffSelection(title: "file.txt", repositoryURL: root, path: "file.txt")
    model.fileReviewSelection = review
    model.isLoading = true

    model.loadRepository(at: root.appendingPathComponent("other"))

    #expect(model.fileReviewSelection?.id == review.id)
    #expect(model.snapshot?.rootPath == original.rootPath)
    #expect(model.isLoading)
}

@Test @MainActor func staleBranchSelectionCannotDeleteOrRenameRecreatedBranch() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let suite = "NiceGitTests-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", startingAt: "HEAD", in: root)
    let model = AppModel(defaults: defaults)
    model.snapshot = try git.loadSnapshot(at: root)
    let stale = try #require(model.snapshot?.branches.first { $0.name == "feature" })
    try "new work\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "New work", in: root)
    try git.deleteBranch("feature", in: root)
    try git.createBranch(named: "feature", startingAt: "HEAD", in: root)
    let recreated = try #require(git.loadSnapshot(at: root).branches.first { $0.name == "feature" })
    #expect(recreated.tip != stale.tip)

    model.deleteBranch(stale)
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(model.errorMessage?.contains("changed") == true)
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "feature" }?.tip == recreated.tip)

    model.errorMessage = nil
    model.renameBranch(stale, to: "renamed")
    let renameDeadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < renameDeadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(!model.isLoading)
    #expect(model.errorMessage?.contains("changed") == true)
    let afterRename = try git.loadSnapshot(at: root).branches
    #expect(afterRename.first { $0.name == "feature" }?.tip == recreated.tip)
    #expect(!afterRename.contains { $0.name == "renamed" })
}

}
