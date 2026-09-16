import Foundation
@testable import NiceGit
import NiceGitCore
import Testing

// Synchronous Git fixture setup must not starve another test's main-actor callbacks.
@Suite(.serialized)
struct AppModelTests {

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
    var successes = 0

    model.commit(message: draft) { draft = ""; successes += 1 }
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(draft.isEmpty)
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
    var successes = 0

    model.commit(message: draft) { draft = ""; successes += 1 }
    let deadline = ContinuousClock.now.advanced(by: .seconds(15))
    while model.isLoading && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }

    #expect(!model.isLoading)
    #expect(draft == "Keep this draft")
    #expect(successes == 0)
    #expect(model.errorMessage != nil)
    #expect(model.errorMessage?.contains("The Git action completed") == false)
    #expect(try git.loadSnapshot(at: root).commits.isEmpty)
}

}
