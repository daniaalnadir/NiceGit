import Foundation
import NiceGitCore
import Testing

@Test func quickStatusMatchesFullStatusForRenamesAndLiteralPaths() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "base\n".write(to: root.appendingPathComponent("old.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try runGit(["mv", "old.txt", "new name.txt"], in: root)
    try "new\n".write(to: root.appendingPathComponent(" leading.txt"), atomically: true, encoding: .utf8)
    let quick = try git.loadStatusWithCheckout(in: root)
    #expect(quick.isComplete)
    #expect(quick.branch == "main")
    #expect(quick.headHash == (try git.loadSnapshot(at: root)).headHash)
    #expect(quick.entries == (try git.loadStatus(in: root)))
}

@Test func graphReferencesPreserveCommasInBranchAndTagNames() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try git.createBranch(named: "feature,one", startingAt: "HEAD", in: root)
    try git.createTag(name: "v1,preview", target: "HEAD", in: root)
    try git.createTag(name: "release,stable", target: "HEAD", message: "Release", in: root)
    try runGit(["update-ref", "refs/remotes/origin/remote,one", "HEAD"], in: root)

    let snapshot = try git.loadSnapshot(at: root)
    let refs = try #require(snapshot.commits.first?.refs)
    #expect(Set(refs) == Set(["HEAD -> main", "feature,one", "origin/remote,one", "tag: v1,preview", "tag: release,stable"]))
}

@Test func commitFileChangeKindsIncludeRootAndDeletedPaths() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    for name in ["edit.txt", "delete.txt"] {
        try "base".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    #expect(try git.commitFileChanges(hash: "HEAD", in: root).map(\.status) == ["A", "A"])
    try "changed".write(to: root.appendingPathComponent("edit.txt"), atomically: true, encoding: .utf8)
    try FileManager.default.removeItem(at: root.appendingPathComponent("delete.txt"))
    try "new".write(to: root.appendingPathComponent("new\nfile.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Changes", in: root)
    let files = try git.commitFileChanges(hash: "HEAD", in: root)
    #expect(files.map(\.path) == ["delete.txt", "edit.txt", "new\nfile.txt"])
    #expect(files.map(\.status) == ["D", "M", "A"])
    let patch = try git.commitFileDiff(hash: "HEAD", path: "edit.txt", in: root)
    #expect(!patch.contains("Author:"))
    #expect(GitDiffLine.changesOnly(patch).map(\.kind) == [.hunk, .deletion, .addition])
}

@Test func commitFileDiffShowsNearbyCodeWithoutCommitMetadata() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("code.swift")
    try "one\ntwo\nthree\nold value\nfive\nsix\nseven\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "one\ntwo\nthree\nnew value\nfive\nsix\nseven\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Edit", in: root)

    let lines = GitDiffLine.codeOnly(try git.commitFileDiff(hash: "HEAD", path: "code.swift", in: root))
    #expect(lines.map(\.kind) == [.hunk, .context, .context, .context, .deletion, .addition, .context, .context, .context])
    #expect(lines[1].text == " one")
    #expect(lines[7].text == " six")
}

@Test func resetModesPreserveOrDiscardChangesAsSelected() throws {
    for mode in GitResetMode.allCases {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let git = GitClient()
        try git.initialize(at: root)
        try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
        let file = root.appendingPathComponent("file.txt")
        try "base\n".write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Base", in: root)
        let base = try #require(git.loadSnapshot(at: root).headHash)
        try "second\n".write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Second", in: root)
        let before = try git.loadSnapshot(at: root)
        let head = try #require(before.headHash)
        try "staged\n".write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try "local\n".write(to: file, atomically: true, encoding: .utf8)
        #expect(throws: (any Error).self) {
            try git.reset(to: base, mode: mode, expectedHead: base, expectedBranch: before.currentBranch, in: root)
        }
        #expect(try git.loadSnapshot(at: root).headHash == head)
        #expect(try String(contentsOf: file, encoding: .utf8) == "local\n")
        try git.reset(to: base, mode: mode, expectedHead: head, expectedBranch: before.currentBranch, in: root)
        #expect(try git.loadSnapshot(at: root).headHash == base)
        #expect(try String(contentsOf: file, encoding: .utf8) == (mode == .hard ? "base\n" : "local\n"))
        let staged = try git.diff(path: "file.txt", staged: true, in: root)
        #expect(mode == .soft ? staged.contains("+staged") : staged.isEmpty)
        let unstaged = try git.diff(path: "file.txt", staged: false, in: root)
        #expect(mode == .hard ? unstaged.isEmpty : unstaged.contains("+local"))
    }
}

@Test func exportedCommitPatchAppliesTextAndBinaryFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let source = root.appendingPathComponent("source")
    let target = root.appendingPathComponent("target")
    try FileManager.default.createDirectory(at: source, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
    let git = GitClient()
    try git.initialize(at: source)
    try git.initialize(at: target)
    try git.setIdentity(name: "Patch Author", email: "patch@example.invalid", in: source)
    try "patch content\n".write(to: source.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    let binary = Data([0, 1, 2, 255, 0, 80])
    try binary.write(to: source.appendingPathComponent("binary.dat"))
    try git.stageAll(in: source)
    try git.commit(message: "Exported commit", in: source)
    let before = try git.loadSnapshot(at: source)
    let patch = try git.exportCommitPatch(hash: #require(before.headHash), in: source)
    #expect(patch.contains("Subject: [PATCH] Exported commit"))
    #expect(patch.contains("GIT binary patch"))
    try git.applyPatch(Data(patch.utf8), in: target)
    #expect(try String(contentsOf: target.appendingPathComponent("file.txt"), encoding: .utf8) == "patch content\n")
    #expect(try Data(contentsOf: target.appendingPathComponent("binary.dat")) == binary)
    #expect(throws: (any Error).self) { try git.applyPatch(Data(patch.utf8), in: target) }
    #expect(try Data(contentsOf: target.appendingPathComponent("binary.dat")) == binary)
    #expect(throws: (any Error).self) { try git.applyPatch(Data("not a patch".utf8), in: target) }
    let traversal = """
    diff --git a/../escaped.txt b/../escaped.txt
    new file mode 100644
    --- /dev/null
    +++ b/../escaped.txt
    @@ -0,0 +1 @@
    +must not escape

    """
    #expect(throws: (any Error).self) { try git.applyPatch(Data(traversal.utf8), in: target) }
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("escaped.txt").path))
    let after = try git.loadSnapshot(at: source)
    #expect(after.headHash == before.headHash)
    #expect(after.status.isEmpty)
    try runGit(["commit", "--allow-empty", "-m", "Empty commit"], in: source)
    #expect(throws: (any Error).self) { try git.exportCommitPatch(hash: "HEAD", in: source) }
}

@Test func integrationRejectsChangedCheckoutBeforeStarting() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let original = try git.loadSnapshot(at: root)
    let head = try #require(original.headHash)
    try runGit(["switch", "-c", "other-checkout"], in: root)
    for operation in [GitOperation.merge, .rebase, .cherryPick, .revert] {
        #expect(throws: (any Error).self) {
            try git.start(operation, target: head, expectedHead: head, expectedBranch: original.currentBranch, in: root)
        }
    }
    let switched = try git.loadSnapshot(at: root)
    #expect(switched.headHash == head)
    #expect(switched.currentBranch == "other-checkout")
    #expect(switched.operation == nil)

    try runGit(["switch", original.currentBranch], in: root)
    try "advanced\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Advanced outside confirmation", in: root)
    let advanced = try git.loadSnapshot(at: root)
    for operation in [GitOperation.merge, .rebase, .cherryPick, .revert] {
        #expect(throws: (any Error).self) {
            try git.start(operation, target: head, expectedHead: head, expectedBranch: original.currentBranch, in: root)
        }
    }
    let after = try git.loadSnapshot(at: root)
    #expect(after.headHash == advanced.headHash)
    #expect(after.operation == nil)
    #expect(after.status.isEmpty)
    try git.start(.merge, target: head, expectedHead: advanced.headHash, expectedBranch: original.currentBranch, in: root)
    #expect(try git.loadSnapshot(at: root).headHash == advanced.headHash)
}

@Test func integrationRejectsMovedSourceBranchBeforeMergeOrRebase() throws {
    // Arrange
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "base\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let original = try git.loadSnapshot(at: root)
    let head = try #require(original.headHash)
    try runGit(["switch", "-c", "feature"], in: root)
    try "first\n".write(to: root.appendingPathComponent("feature.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "First", in: root)
    let selectedTip = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["switch", original.currentBranch], in: root)
    try runGit(["update-ref", "refs/remotes/origin/feature", selectedTip], in: root)
    let local = GitBranch(name: "feature", isCurrent: false, isRemote: false, tip: selectedTip, subject: "First")
    let remote = GitBranch(name: "remotes/origin/feature", isCurrent: false, isRemote: true, tip: selectedTip, subject: "First")
    try runGit(["switch", "feature"], in: root)
    try "second\n".write(to: root.appendingPathComponent("feature.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Second", in: root)
    let newerTip = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["switch", original.currentBranch], in: root)
    try runGit(["update-ref", "refs/remotes/origin/feature", newerTip], in: root)

    // Act
    for source in [local, remote] {
        for operation in [GitOperation.merge, .rebase] {
            #expect(throws: (any Error).self) {
                try git.start(operation, target: selectedTip, expectedHead: head, expectedBranch: original.currentBranch, expectedSourceBranch: source, in: root)
            }
        }
    }

    // Assert
    let after = try git.loadSnapshot(at: root)
    #expect(after.headHash == head)
    #expect(after.currentBranch == original.currentBranch)
    #expect(after.operation == nil)
    #expect(after.status.isEmpty)
}

@Test func stashPopRestoresChangesAndKeepsStashOnFailure() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "staged\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try "unstaged\n".write(to: file, atomically: true, encoding: .utf8)
    let untracked = root.appendingPathComponent("new.txt")
    try "new\n".write(to: untracked, atomically: true, encoding: .utf8)
    let staged = try git.diff(path: "file.txt", staged: true, in: root)
    let unstaged = try git.diff(path: "file.txt", staged: false, in: root)
    try git.saveStash(message: "Saved", includeUntracked: true, in: root)
    let stash = try #require(git.listStashes(in: root).first)
    #expect(try git.stashFiles(hash: stash.hash, in: root) == ["file.txt", "new.txt"])
    try git.popStash(stash, in: root)
    #expect(try git.listStashes(in: root).isEmpty)
    #expect(try git.diff(path: "file.txt", staged: true, in: root) == staged)
    #expect(try git.diff(path: "file.txt", staged: false, in: root) == unstaged)
    #expect(try String(contentsOf: untracked, encoding: .utf8) == "new\n")
    #expect(throws: (any Error).self) { try git.popStash(stash, in: root) }

    try git.saveStash(message: "Conflict candidate", includeUntracked: true, in: root)
    let conflicting = try #require(git.listStashes(in: root).first)
    try "different committed content\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Conflicting change", in: root)
    #expect(throws: (any Error).self) { try git.popStash(conflicting, in: root) }
    #expect(try git.listStashes(in: root).contains { $0.hash == conflicting.hash })
}

@Test func deletedStashCannotBeAppliedFromStaleSelection() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "saved\n".write(to: file, atomically: true, encoding: .utf8)
    try git.saveStash(message: "Saved", includeUntracked: false, in: root)
    let stale = try #require(git.listStashes(in: root).first)
    try git.dropStash(stale, in: root)

    #expect(throws: (any Error).self) { try git.applyStash(stale, in: root) }
    #expect(try String(contentsOf: file, encoding: .utf8) == "base\n")
    #expect(try git.loadStatus(in: root).isEmpty)
}

@Test func stashReportsWhenUntrackedFilesWereExcludedAndNothingWasSaved() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let file = root.appendingPathComponent("untracked.txt")
    try "local\n".write(to: file, atomically: true, encoding: .utf8)

    #expect(throws: (any Error).self) {
        try git.saveStash(message: "Excluded", includeUntracked: false, in: root)
    }
    #expect(try git.listStashes(in: root).isEmpty)
    #expect(try String(contentsOf: file, encoding: .utf8) == "local\n")
}

@Test func messageAmendPreservesStagedAndUnstagedChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Original", in: root)
    let oldHead = try #require(git.loadSnapshot(at: root).headHash)
    try "staged\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try "unstaged\n".write(to: file, atomically: true, encoding: .utf8)
    let staged = try git.diff(path: "file.txt", staged: true, in: root)
    let unstaged = try git.diff(path: "file.txt", staged: false, in: root)
    try git.amendMessage("Revised\n\nDetailed body", expectedHead: oldHead, in: root)
    let after = try git.loadSnapshot(at: root)
    #expect(after.headHash != oldHead)
    #expect(after.commits.count == 1)
    #expect(try git.commitMessage(hash: "HEAD", in: root).contains("Detailed body"))
    #expect(try git.diff(path: "file.txt", staged: true, in: root) == staged)
    #expect(try git.diff(path: "file.txt", staged: false, in: root) == unstaged)
    #expect(throws: (any Error).self) { try git.amendMessage("Stale edit", expectedHead: oldHead, in: root) }
    #expect(try git.loadSnapshot(at: root).headHash == after.headHash)
}

@Test func remoteCheckoutReusesTrackingBranchWithoutResettingItsCommits() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["remote", "add", "origin", root.path], in: root)
    try runGit(["update-ref", "refs/remotes/origin/feature", "HEAD"], in: root)
    try git.checkoutRemote(branch: "remotes/origin/feature", in: root)
    #expect(try git.loadSnapshot(at: root).currentBranch == "feature")
    try runGit(["commit", "--allow-empty", "-m", "Local work"], in: root)
    let localTip = try #require(git.loadSnapshot(at: root).headHash)
    try git.renameBranch("feature", to: "renamed-feature", in: root)
    try git.checkout(branch: "main", in: root)
    try git.checkoutRemote(branch: "refs/remotes/origin/feature", in: root)
    let snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.currentBranch == "renamed-feature")
    #expect(snapshot.headHash == localTip)
    try runGit(["branch", "--track", "second", "origin/feature"], in: root)
    try git.checkout(branch: "main", in: root)
    #expect(throws: (any Error).self) { try git.checkoutRemote(branch: "origin/feature", in: root) }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
}

@Test func remoteCheckoutUsesDistinctLocalNameWhenRemotesShareBranchName() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["remote", "add", "origin", root.path], in: root)
    try runGit(["remote", "add", "upstream", root.path], in: root)
    try runGit(["update-ref", "refs/remotes/origin/feature", "HEAD"], in: root)
    try runGit(["update-ref", "refs/remotes/upstream/feature", "HEAD"], in: root)

    try git.checkoutRemote(branch: "origin/feature", in: root)
    try git.checkout(branch: "main", in: root)
    try git.checkoutRemote(branch: "upstream/feature", in: root)
    let snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.currentBranch == "upstream-feature")
    #expect(snapshot.upstream == "upstream/feature")
    #expect(snapshot.branches.contains { !$0.isRemote && $0.name == "feature" && $0.upstream == "refs/remotes/origin/feature" })
}

@Test func remoteHeadAliasesAreNotShownAsBranches() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let head = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["remote", "add", "origin", root.path], in: root)
    try runGit(["update-ref", "refs/remotes/team/shared/main", head], in: root)
    try runGit(["symbolic-ref", "refs/remotes/team/shared/HEAD", "refs/remotes/team/shared/main"], in: root)
    try runGit(["update-ref", "refs/remotes/origin/topic/HEAD", head], in: root)

    let branches = try git.loadSnapshot(at: root).branches
    #expect(branches.contains { $0.name == "remotes/team/shared/main" })
    #expect(branches.contains { $0.name == "remotes/origin/topic/HEAD" })
    #expect(branches.allSatisfy { $0.name != "remotes/team/shared/HEAD" })
    #expect(throws: (any Error).self) {
        try git.checkoutRemote(branch: "remotes/team/shared/HEAD", expectedTip: head, in: root)
    }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    try git.checkoutRemote(branch: "remotes/origin/topic/HEAD", expectedTip: head, in: root)
    #expect(try git.loadSnapshot(at: root).currentBranch == "topic/HEAD")
}

@Test func linkedWorktreeWithNewlinePathDetectsGitOperation() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("repository\nwith newline")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "base\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let head = try #require(git.loadSnapshot(at: root).headHash)
    try git.createBranch(named: "feature", startingAt: head, in: root)
    let linked = root.appendingPathComponent("linked\ncheckout")
    try git.createWorktree(branch: "feature", at: linked, in: root)
    let metadata = try String(contentsOf: linked.appendingPathComponent(".git"), encoding: .utf8)
    #expect(metadata.hasPrefix("gitdir: "))
    let gitDirectory = String(metadata.dropFirst("gitdir: ".count).dropLast())
    try (head + "\n").write(to: URL(fileURLWithPath: gitDirectory).appendingPathComponent("MERGE_HEAD"), atomically: true, encoding: .utf8)

    #expect(try git.loadSnapshot(at: linked).operation == .merge)
}

@Test func switchingBranchesSavesStagedUnstagedAndUntrackedChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    let untracked = root.appendingPathComponent("new.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try runGit(["switch", "-c", "feature"], in: root)
    try "feature\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Feature", in: root)
    try "staged\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try "unstaged\n".write(to: file, atomically: true, encoding: .utf8)
    try "untracked\n".write(to: untracked, atomically: true, encoding: .utf8)

    #expect(try git.checkout(branch: "main", in: root))
    let switched = try git.loadSnapshot(at: root)
    #expect(switched.currentBranch == "main")
    #expect(switched.status.isEmpty)
    #expect(try String(contentsOf: file, encoding: .utf8) == "base\n")
    #expect(!FileManager.default.fileExists(atPath: untracked.path))
    let stash = try #require(switched.stashes.first)
    #expect(stash.message.contains("feature before switching to main"))
    #expect(try !git.checkout(branch: "feature", in: root))
    try git.applyStash(stash, in: root)
    #expect(try String(contentsOf: file, encoding: .utf8) == "unstaged\n")
    #expect(try String(contentsOf: untracked, encoding: .utf8) == "untracked\n")
    #expect(try git.diff(path: "file.txt", staged: true, in: root).contains("+staged"))
    #expect(try git.diff(path: "file.txt", staged: false, in: root).contains("+unstaged"))
}

@Test func failedBranchSwitchRestoresChangesAndDoesNotLeaveAStash() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "changed\n".write(to: file, atomically: true, encoding: .utf8)

    #expect(throws: (any Error).self) { try git.checkout(branch: "missing", in: root) }
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "main")
    #expect(after.stashes.isEmpty)
    #expect(after.status.count == 1)
    #expect(try String(contentsOf: file, encoding: .utf8) == "changed\n")
}

@Test func branchSwitchRejectsSelectedLocalAndRemoteTipsThatMoved() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["branch", "feature"], in: root)
    try runGit(["update-ref", "refs/remotes/origin/remote-feature", "HEAD"], in: root)
    let selected = try git.loadSnapshot(at: root)
    let oldLocal = try #require(selected.branches.first { $0.name == "feature" }?.tip)
    let oldRemote = try #require(selected.branches.first { $0.name == "remotes/origin/remote-feature" }?.tip)
    try runGit(["commit", "--allow-empty", "-m", "Advanced main"], in: root)
    try runGit(["branch", "--force", "feature", "main"], in: root)
    try runGit(["update-ref", "refs/remotes/origin/remote-feature", "HEAD"], in: root)
    let current = try git.loadSnapshot(at: root)

    #expect(throws: (any Error).self) {
        try git.checkout(branch: "feature", expectedTip: oldLocal, in: root)
    }
    #expect(throws: (any Error).self) {
        try git.checkoutRemote(branch: "remotes/origin/remote-feature", expectedTip: oldRemote, in: root)
    }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    #expect(try git.loadSnapshot(at: root).stashes.isEmpty)
    #expect(try git.loadSnapshot(at: root).branches.allSatisfy { $0.name != "remote-feature" })
    let newLocal = try #require(current.branches.first { $0.name == "feature" }?.tip)
    try git.checkout(branch: "feature", expectedTip: newLocal, in: root)
    #expect(try git.loadSnapshot(at: root).currentBranch == "feature")
}

@Test func branchSwitchRejectsChangedStartingCheckoutBeforeStashing() throws {
    // Arrange
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["branch", "feature"], in: root)
    try runGit(["update-ref", "refs/remotes/origin/remote-feature", "HEAD"], in: root)
    let selected = try git.loadSnapshot(at: root)
    let head = try #require(selected.headHash)
    let localTip = try #require(selected.branches.first { $0.name == "feature" }?.tip)
    let remoteTip = try #require(selected.branches.first { $0.name == "remotes/origin/remote-feature" }?.tip)
    try runGit(["switch", "-c", "other"], in: root)
    let draft = root.appendingPathComponent("draft.txt")
    try "Keep this work\n".write(to: draft, atomically: true, encoding: .utf8)

    // Act
    #expect(throws: (any Error).self) {
        try git.checkout(branch: "feature", expectedTip: localTip, expectedCurrentBranch: selected.currentBranch, expectedHead: head, in: root)
    }
    #expect(throws: (any Error).self) {
        try git.checkoutRemote(branch: "remotes/origin/remote-feature", expectedTip: remoteTip, expectedCurrentBranch: selected.currentBranch, expectedHead: head, in: root)
    }

    // Assert
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "other")
    #expect(after.stashes.isEmpty)
    #expect(try String(contentsOf: draft, encoding: .utf8) == "Keep this work\n")
    try FileManager.default.removeItem(at: draft)
    try runGit(["switch", selected.currentBranch], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Advance HEAD"], in: root)
    #expect(throws: (any Error).self) {
        try git.checkout(branch: "feature", expectedTip: localTip, expectedCurrentBranch: selected.currentBranch, expectedHead: head, in: root)
    }
    let current = try git.loadSnapshot(at: root)
    #expect(current.currentBranch == selected.currentBranch)
    #expect(current.stashes.isEmpty)
    #expect(try !git.checkout(branch: "feature", expectedTip: localTip, expectedCurrentBranch: current.currentBranch, expectedHead: current.headHash, in: root))
    #expect(try git.loadSnapshot(at: root).currentBranch == "feature")
}

@Test func branchSwitchKeepsDirtySubmoduleAndExistingStash() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let source = base.appendingPathComponent("source")
    let root = base.appendingPathComponent("checkout")
    try FileManager.default.createDirectory(at: source, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: source)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: source)
    try "base\n".write(to: source.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: source)
    try git.commit(message: "Submodule base", in: source)

    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let topLevel = root.appendingPathComponent("top.txt")
    try "base\n".write(to: topLevel, atomically: true, encoding: .utf8)
    try runGit(["-c", "protocol.file.allow=always", "submodule", "add", source.path, "nested"], in: root)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try runGit(["branch", "feature"], in: root)

    try "earlier\n".write(to: root.appendingPathComponent("earlier.txt"), atomically: true, encoding: .utf8)
    try git.saveStash(message: "Earlier work", includeUntracked: true, in: root)
    let earlierStash = try #require(git.listStashes(in: root).first)
    let nestedFile = root.appendingPathComponent("nested/file.txt")
    try "submodule edit\n".write(to: nestedFile, atomically: true, encoding: .utf8)
    let nestedEntry = try #require(git.loadStatus(in: root).first { $0.path == "nested" })
    do {
        try git.discard(nestedEntry, in: root)
        Issue.record("Discard reported success while submodule changes remained")
    } catch {
        #expect(error.localizedDescription.contains("submodule"))
    }
    #expect(try String(contentsOf: nestedFile, encoding: .utf8) == "submodule edit\n")

    #expect(throws: (any Error).self) { try git.checkout(branch: "feature", in: root) }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    #expect(try String(contentsOf: nestedFile, encoding: .utf8) == "submodule edit\n")
    #expect(try git.listStashes(in: root).map(\.hash) == [earlierStash.hash])
    #expect(throws: (any Error).self) {
        try git.saveStash(message: "Submodule only", includeUntracked: true, in: root)
    }
    #expect(try git.listStashes(in: root).map(\.hash) == [earlierStash.hash])

    try "top-level edit\n".write(to: topLevel, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.checkout(branch: "feature", in: root) }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    #expect(try String(contentsOf: nestedFile, encoding: .utf8) == "submodule edit\n")
    #expect(try String(contentsOf: topLevel, encoding: .utf8) == "top-level edit\n")
    #expect(try git.listStashes(in: root).map(\.hash) == [earlierStash.hash])
    #expect(throws: (any Error).self) {
        try git.saveStash(message: "Partial stash", includeUntracked: true, in: root)
    }
    #expect(try git.listStashes(in: root).count == 2)
    #expect(try String(contentsOf: nestedFile, encoding: .utf8) == "submodule edit\n")
    #expect(try String(contentsOf: topLevel, encoding: .utf8) == "base\n")
}

@Test(arguments: ["checkout", "merge", "merge-diverged"], ["generated.txt", "output/generated.txt", "[cache].txt"])
func checkoutAndMergePreserveIgnoredLocalFiles(action: String, path: String) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try ((path.hasPrefix("output/") ? "output/" : path.replacingOccurrences(of: "[", with: "\\[")) + "\n").write(to: root.appendingPathComponent(".gitignore"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Ignore generated file", in: root)
    try git.createBranch(named: "feature", in: root)
    let ignoredFile = root.appendingPathComponent(path)
    try FileManager.default.createDirectory(at: ignoredFile.deletingLastPathComponent(), withIntermediateDirectories: true)
    try "tracked feature\n".write(to: ignoredFile, atomically: true, encoding: .utf8)
    try runGit(["add", "--force", "--", path], in: root)
    try git.commit(message: "Track generated file", in: root)
    try git.checkout(branch: "main", in: root)
    if action == "merge-diverged" {
        try "main work\n".write(to: root.appendingPathComponent("main.txt"), atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Independent main work", in: root)
    }
    let before = try git.loadSnapshot(at: root).headHash
    try FileManager.default.createDirectory(at: ignoredFile.deletingLastPathComponent(), withIntermediateDirectories: true)
    try "ignored local\n".write(to: ignoredFile, atomically: true, encoding: .utf8)
    #expect(try git.loadStatus(in: root).isEmpty)

    #expect(throws: (any Error).self) {
        if action == "checkout" { try git.checkout(branch: "feature", in: root) }
        else { try git.start(.merge, target: "feature", in: root) }
    }
    #expect(try git.loadSnapshot(at: root).headHash == before)
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    #expect(try String(contentsOf: ignoredFile, encoding: .utf8) == "ignored local\n")
    try FileManager.default.removeItem(at: ignoredFile)
    if path.hasPrefix("output/") {
        try "unrelated local\n".write(to: ignoredFile.deletingLastPathComponent().appendingPathComponent("other.txt"), atomically: true, encoding: .utf8)
    }
    if action == "checkout" { try git.checkout(branch: "feature", in: root) }
    else { try git.start(.merge, target: "feature", in: root) }
    #expect(try String(contentsOf: ignoredFile, encoding: .utf8) == "tracked feature\n")
    if path.hasPrefix("output/") {
        #expect(try String(contentsOf: ignoredFile.deletingLastPathComponent().appendingPathComponent("other.txt"), encoding: .utf8) == "unrelated local\n")
    }
}

@Test func discardRemovesStagedUnstagedRenamedAndUntrackedChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let edited = root.appendingPathComponent("edited.txt")
    let deleted = root.appendingPathComponent("deleted.txt")
    let oldName = root.appendingPathComponent("old.txt")
    let newName = root.appendingPathComponent("new.txt")
    let untracked = root.appendingPathComponent("untracked\nfile.txt")
    for file in [edited, deleted, oldName] {
        try "base\n".write(to: file, atomically: true, encoding: .utf8)
    }
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)

    try "staged\n".write(to: edited, atomically: true, encoding: .utf8)
    try git.stage(path: "edited.txt", in: root)
    try "unstaged\n".write(to: edited, atomically: true, encoding: .utf8)
    try runGit(["rm", "deleted.txt"], in: root)
    try runGit(["mv", "old.txt", "new.txt"], in: root)
    try "new\n".write(to: untracked, atomically: true, encoding: .utf8)
    let entries = try git.loadStatus(in: root)
    #expect(entries.count == 4)

    for entry in entries {
        try git.discard(entry, in: root)
    }

    #expect(try git.loadStatus(in: root).isEmpty)
    for file in [edited, deleted, oldName] {
        #expect(try String(contentsOf: file, encoding: .utf8) == "base\n")
    }
    #expect(!FileManager.default.fileExists(atPath: newName.path))
    #expect(!FileManager.default.fileExists(atPath: untracked.path))
}

@Test func discardBeforeFirstCommitAndStaleSelectionPreservesFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    let staged = root.appendingPathComponent("staged.txt")
    let untracked = root.appendingPathComponent("untracked.txt")
    try "staged\n".write(to: staged, atomically: true, encoding: .utf8)
    try git.stage(path: "staged.txt", in: root)
    try "untracked\n".write(to: untracked, atomically: true, encoding: .utf8)
    let selected = try #require(git.loadStatus(in: root).first { $0.path == "staged.txt" })
    try "changed\n".write(to: staged, atomically: true, encoding: .utf8)

    #expect(throws: (any Error).self) { try git.discard(selected, in: root) }
    #expect(try String(contentsOf: staged, encoding: .utf8) == "changed\n")
    for entry in try git.loadStatus(in: root) {
        try git.discard(entry, in: root)
    }
    #expect(try git.loadStatus(in: root).isEmpty)
    #expect(!FileManager.default.fileExists(atPath: staged.path))
    #expect(!FileManager.default.fileExists(atPath: untracked.path))

    let nested = root.appendingPathComponent("nested")
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    try runGit(["init", "--initial-branch=main"], in: nested)
    let nestedFile = nested.appendingPathComponent("file.txt")
    try "nested work\n".write(to: nestedFile, atomically: true, encoding: .utf8)
    let nestedEntry = try #require(git.loadStatus(in: root).first { $0.path == "nested/" })
    #expect(throws: (any Error).self) { try git.discard(nestedEntry, in: root) }
    #expect(try String(contentsOf: nestedFile, encoding: .utf8) == "nested work\n")
}

@Test func discardUntrackedGlobNameDoesNotRemoveMatchingFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    let selectedFile = root.appendingPathComponent("[ab].txt")
    let unrelatedFile = root.appendingPathComponent("a.txt")
    try "selected\n".write(to: selectedFile, atomically: true, encoding: .utf8)
    try "unrelated\n".write(to: unrelatedFile, atomically: true, encoding: .utf8)
    let selected = try #require(git.loadStatus(in: root).first { $0.path == "[ab].txt" })

    try git.discard(selected, in: root)

    #expect(!FileManager.default.fileExists(atPath: selectedFile.path))
    #expect(try String(contentsOf: unrelatedFile, encoding: .utf8) == "unrelated\n")
    #expect(try git.loadStatus(in: root).map(\.path) == ["a.txt"])
}

@Test func copiedFileActionsPreserveChangesInTheSource() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["config", "status.renames", "copies"], in: root)
    let source = root.appendingPathComponent("source.txt")
    let copy = root.appendingPathComponent("copy.txt")
    try "one\ntwo\nthree\nfour\nfive\n".write(to: source, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try FileManager.default.copyItem(at: source, to: copy)
    try "one\ntwo\nthree\nfour\nupdated\n".write(to: source, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    let selected = try #require(git.loadStatus(in: root).first { $0.path == "copy.txt" })
    #expect(selected.kind == .added)
    #expect(selected.originalPath == "source.txt")
    let copyDiff = try git.diff(path: selected.path, staged: true, originalPath: selected.originalPath, in: root)
    #expect(copyDiff.contains("diff --git a/copy.txt b/copy.txt"))
    #expect(!copyDiff.contains("diff --git a/source.txt b/source.txt"))

    try git.unstage(path: selected.path, originalPath: selected.originalPath, in: root)
    #expect(try git.loadStatus(in: root).first { $0.path == "source.txt" }?.indexStatus == "M")
    #expect(try git.loadStatus(in: root).first { $0.path == "copy.txt" }?.kind == .untracked)
    try git.stage(path: selected.path, in: root)
    let copiedAgain = try #require(git.loadStatus(in: root).first { $0.path == "copy.txt" })
    try git.discard(copiedAgain, in: root)
    #expect(!FileManager.default.fileExists(atPath: copy.path))
    #expect(try String(contentsOf: source, encoding: .utf8).hasSuffix("updated\n"))
    #expect(try git.loadStatus(in: root).map(\.path) == ["source.txt"])
    #expect(try git.diff(path: "source.txt", staged: true, in: root).contains("+updated"))
}

@Test func selectedBranchPushDoesNotPushHeadOrForceRemote() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let remote = base.appendingPathComponent("remote.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try runGit(["init", "--bare"], in: remote)
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["branch", "feature"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Newer main"], in: root)
    try runGit(["remote", "add", "origin", remote.path], in: root)
    try runGit(["config", "remote.origin.mirror", "true"], in: root)
    let before = try git.loadSnapshot(at: root)
    try git.pushBranch("feature", to: "origin", in: root)
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).trimmingCharacters(in: .newlines) == "Base")
    let selectedTip = try #require(before.branches.first { $0.name == "feature" }?.tip)
    try runGit(["branch", "--force", "feature", "main"], in: root)
    #expect(throws: (any Error).self) {
        try git.pushBranch("feature", to: "origin", expectedTip: selectedTip, in: root)
    }
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).trimmingCharacters(in: .newlines) == "Base")
    try runGit(["branch", "--force", "feature", selectedTip], in: root)
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/heads/main"], in: remote) }
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "main")
    #expect(after.headHash == before.headHash)
    #expect(after.branches.first { $0.name == "feature" }?.upstream == nil)
    try runGit(["-c", "remote.origin.mirror=false", "push", "origin", "main:feature"], in: root)
    #expect(throws: (any Error).self) { try git.pushBranch("feature", to: "origin", in: root) }
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).contains("Newer main"))
    #expect(throws: (any Error).self) { try git.pushBranch("missing", to: "origin", in: root) }
}

@Test func currentBranchPushIgnoresMatchingBranchesMirrorAndTags() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let remote = base.appendingPathComponent("remote.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try runGit(["init", "--bare"], in: remote)
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["remote", "add", "origin", remote.path], in: root)
    try runGit(["push", "--set-upstream", "origin", "main"], in: root)
    try git.createBranch(named: "feature", in: root)
    try runGit(["push", "--set-upstream", "origin", "feature"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Unpushed feature"], in: root)
    try git.checkout(branch: "main", in: root)
    let main = try git.loadSnapshot(at: root)
    try git.createTag(name: "unwanted", target: "HEAD", message: "Do not push", in: root)
    try runGit(["config", "push.default", "matching"], in: root)
    try runGit(["config", "push.followTags", "true"], in: root)
    try runGit(["config", "remote.origin.mirror", "true"], in: root)

    try git.push(expectedBranch: "main", expectedHead: main.headHash, expectedUpstream: main.upstream, in: root)
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).contains("Base"))
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/tags/unwanted"], in: remote) }
    try runGit(["update-ref", "refs/remotes/origin/other", try #require(main.headHash)], in: root)
    try runGit(["branch", "--set-upstream-to=origin/other", "main"], in: root)
    #expect(throws: (any Error).self) {
        try git.push(expectedBranch: "main", expectedHead: main.headHash, expectedUpstream: main.upstream, in: root)
    }
    try runGit(["branch", "--set-upstream-to=origin/main", "main"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "New main"], in: root)
    #expect(throws: (any Error).self) {
        try git.push(expectedBranch: "main", expectedHead: main.headHash, in: root)
    }
    #expect(try git.commitMessage(hash: "refs/heads/main", in: remote).contains("Base"))
    try git.push(expectedBranch: "main", expectedHead: git.loadSnapshot(at: root).headHash, in: root)
    #expect(try git.commitMessage(hash: "refs/heads/main", in: remote).contains("New main"))
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).contains("Base"))
    try git.createBranch(named: "published", in: root)
    let selectedPublishedHead = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["commit", "--allow-empty", "-m", "Later published work"], in: root)
    #expect(throws: (any Error).self) {
        try git.publish(remote: "origin", expectedBranch: "published", expectedHead: selectedPublishedHead, in: root)
    }
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/heads/published"], in: remote) }
    try git.publish(remote: "origin", expectedBranch: "published", expectedHead: git.loadSnapshot(at: root).headHash, in: root)
    #expect(try git.commitMessage(hash: "refs/heads/published", in: remote).contains("Later published work"))
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).contains("Base"))
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/tags/unwanted"], in: remote) }
    #expect(try git.loadSnapshot(at: root).upstream == "origin/published")
}

@Test func pullRejectsChangedCheckoutBeforeFastForward() throws {
    // Arrange
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let remote = base.appendingPathComponent("remote.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try runGit(["init", "--bare"], in: remote)
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let selected = try git.loadSnapshot(at: root)
    let selectedHead = try #require(selected.headHash)
    try runGit(["remote", "add", "origin", remote.path], in: root)
    try runGit(["push", "--set-upstream", "origin", "main"], in: root)
    let selectedUpstream = try #require(git.loadSnapshot(at: root).upstream)
    try runGit(["commit", "--allow-empty", "-m", "Remote advancement"], in: root)
    let remoteHead = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["push", "origin", "main"], in: root)
    try runGit(["reset", "--hard", selectedHead], in: root)
    try runGit(["switch", "-c", "other"], in: root)

    // Act
    #expect(throws: (any Error).self) {
        try git.pull(expectedBranch: selected.currentBranch, expectedHead: selectedHead, expectedUpstream: selectedUpstream, in: root)
    }
    try runGit(["switch", selected.currentBranch], in: root)
    try runGit(["update-ref", "refs/remotes/origin/other", selectedHead], in: root)
    try runGit(["branch", "--set-upstream-to=origin/other", selected.currentBranch], in: root)
    #expect(throws: (any Error).self) {
        try git.pull(expectedBranch: selected.currentBranch, expectedHead: selectedHead, expectedUpstream: selectedUpstream, in: root)
    }
    try runGit(["branch", "--set-upstream-to=origin/main", selected.currentBranch], in: root)
    try git.pull(expectedBranch: selected.currentBranch, expectedHead: selectedHead, expectedUpstream: selectedUpstream, in: root)
    #expect(throws: (any Error).self) {
        try git.pull(expectedBranch: selected.currentBranch, expectedHead: selectedHead, expectedUpstream: selectedUpstream, in: root)
    }

    // Assert
    let after = try git.loadSnapshot(at: root)
    #expect(after.headHash == remoteHead)
    #expect(after.currentBranch == selected.currentBranch)
    #expect(after.status.isEmpty)
}

@Test func remoteActionsRejectChangedDestination() throws {
    // Arrange
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let originalRemote = base.appendingPathComponent("original.git")
    let replacementRemote = base.appendingPathComponent("replacement.git")
    for directory in [root, originalRemote, replacementRemote] {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try runGit(["init", "--bare"], in: originalRemote)
    try runGit(["init", "--bare"], in: replacementRemote)
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["remote", "add", "origin", originalRemote.path], in: root)
    try runGit(["push", "--set-upstream", "origin", "main"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Unpushed work"], in: root)
    let selected = try git.loadSnapshot(at: root)
    let head = try #require(selected.headHash)
    try runGit(["remote", "set-url", "origin", replacementRemote.path], in: root)

    // Act
    for action in ["pull", "push", "pushBranch", "publish"] {
        do {
            switch action {
            case "pull":
                try git.pull(expectedBranch: selected.currentBranch, expectedHead: head, expectedUpstream: selected.upstream, expectedFetchAddresses: selected.remoteFetchAddresses, in: root)
            case "push":
                try git.push(expectedBranch: selected.currentBranch, expectedHead: head, expectedUpstream: selected.upstream, expectedPushAddresses: selected.remotePushAddresses, in: root)
            case "pushBranch":
                try git.pushBranch("main", to: "origin", expectedTip: head, expectedPushAddresses: selected.remotePushAddresses, in: root)
            default:
                try git.publish(remote: "origin", expectedBranch: selected.currentBranch, expectedHead: head, expectedPushAddresses: selected.remotePushAddresses, in: root)
            }
            Issue.record("\(action) accepted a changed remote address")
        } catch {
            #expect(error.localizedDescription.contains("remote address changed"))
        }
    }

    // Assert
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/heads/main"], in: replacementRemote) }
    #expect(try git.commitMessage(hash: "refs/heads/main", in: originalRemote).contains("Base"))
    try runGit(["remote", "set-url", "origin", originalRemote.path], in: root)
    try git.pull(expectedBranch: selected.currentBranch, expectedHead: head, expectedUpstream: selected.upstream, expectedFetchAddresses: selected.remoteFetchAddresses, in: root)
    try git.pushBranch("main", to: "origin", expectedTip: head, expectedPushAddresses: selected.remotePushAddresses, in: root)
    try git.push(expectedBranch: selected.currentBranch, expectedHead: head, expectedUpstream: selected.upstream, expectedPushAddresses: selected.remotePushAddresses, in: root)
    try git.publish(remote: "origin", expectedBranch: selected.currentBranch, expectedHead: head, expectedPushAddresses: selected.remotePushAddresses, in: root)
    #expect(try git.commitMessage(hash: "refs/heads/main", in: originalRemote).contains("Unpushed work"))
}

@Test func revertPreservesHistoryAndSupportsConflictAbortAndContinue() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "change\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Change", in: root)
    let change = try #require(git.loadSnapshot(at: root).headHash)
    try git.start(.revert, target: change, in: root)
    #expect(try String(contentsOf: file, encoding: .utf8) == "base\n")
    #expect(try git.loadSnapshot(at: root).commits.count == 3)
    try "later\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Later", in: root)
    let before = try git.loadSnapshot(at: root)
    #expect(throws: (any Error).self) { try git.start(.revert, target: change, in: root) }
    #expect(try git.loadSnapshot(at: root).operation == .revert)
    try git.abortOperation(.revert, in: root)
    #expect(try git.loadSnapshot(at: root).headHash == before.headHash)
    #expect(try String(contentsOf: file, encoding: .utf8) == "later\n")
    #expect(throws: (any Error).self) { try git.start(.revert, target: change, in: root) }
    try "resolved\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.continueOperation(.revert, in: root)
    #expect(try git.loadSnapshot(at: root).operation == nil)
    #expect(try git.loadSnapshot(at: root).commits.count == 5)
}

@Test func annotatedTagsPreserveMessageAndSelectedCommit() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Tag Author", email: "tag@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Selected release"], in: root)
    let selected = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["commit", "--allow-empty", "-m", "Later work"], in: root)
    try git.createTag(name: "v1", target: selected, message: "Release notes\n\nDetailed changes", in: root)
    try runGit(["cat-file", "-e", "refs/tags/v1^{tag}"], in: root)
    let details = try git.commitDiff(hash: "refs/tags/v1", in: root)
    #expect(details.contains("Release notes"))
    #expect(details.contains("Detailed changes"))
    #expect(details.contains("Tag Author"))
    #expect(details.contains(selected))
    #expect(throws: (any Error).self) { try git.createTag(name: "v1", target: "HEAD", message: "Replacement", in: root) }
    #expect(throws: (any Error).self) { try git.createTag(name: "empty", target: selected, message: " \n", in: root) }
    try git.createTag(name: "light", target: selected, in: root)
    #expect(throws: (any Error).self) { try runGit(["cat-file", "-e", "refs/tags/light^{tag}"], in: root) }
    #expect(try git.loadSnapshot(at: root).tags.sorted() == ["light", "v1"])
    let originalTip = try #require(git.loadSnapshot(at: root).tagTips["v1"])
    try runGit(["tag", "--delete", "v1"], in: root)
    try git.createTag(name: "v1", target: "HEAD", message: "Replacement release", in: root)
    let replacementTip = try #require(git.loadSnapshot(at: root).tagTips["v1"])
    #expect(replacementTip != originalTip)
    #expect(throws: (any Error).self) { try git.deleteTag(name: "v1", expectedTip: originalTip, in: root) }
    #expect(try git.loadSnapshot(at: root).tagTips["v1"] == replacementTip)
    try git.deleteTag(name: "v1", expectedTip: replacementTip, in: root)
    #expect(try git.loadSnapshot(at: root).tags == ["light"])
}

@Test func upstreamChangesTargetSelectedBranchWithoutCheckout() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["branch", "feature"], in: root)
    try runGit(["remote", "add", "origin", root.path], in: root)
    try runGit(["update-ref", "refs/remotes/origin/main", "HEAD"], in: root)
    let before = try git.loadSnapshot(at: root)
    let selectedTip = try #require(before.branches.first { $0.name == "feature" }?.tip)
    try runGit(["commit", "--allow-empty", "-m", "Later"], in: root)
    try runGit(["branch", "--force", "feature", "HEAD"], in: root)
    #expect(throws: (any Error).self) {
        try git.setUpstream(branch: "feature", remoteBranch: "origin/main", expectedTip: selectedTip, in: root)
    }
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "feature" }?.upstream == nil)
    let currentTip = try #require(git.loadSnapshot(at: root).branches.first { $0.name == "feature" }?.tip)
    try git.setUpstream(branch: "feature", remoteBranch: "origin/main", expectedTip: currentTip, in: root)
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "main")
    #expect(after.headHash != before.headHash)
    #expect(after.upstream == nil)
    #expect(after.branches.first { $0.name == "feature" }?.upstream == "refs/remotes/origin/main")
    #expect(throws: (any Error).self) { try git.setUpstream(branch: "feature", remoteBranch: "origin/missing", in: root) }
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "feature" }?.upstream == "refs/remotes/origin/main")
    try git.setUpstream(branch: "feature", remoteBranch: nil, in: root)
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "feature" }?.upstream == nil)
}

@Test func createBranchAtSelectedCommitPreservesCheckoutAndChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try runGit(["init", "--initial-branch=main"], in: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let base = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["commit", "--allow-empty", "-m", "Newer"], in: root)
    let file = root.appendingPathComponent("draft.txt")
    try "Unsaved work".write(to: file, atomically: true, encoding: .utf8)
    let before = try git.loadSnapshot(at: root)
    try git.createBranch(named: "selected-tip", startingAt: base, in: root)
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "main")
    #expect(after.headHash == before.headHash)
    #expect(after.status == before.status)
    #expect(after.branches.first { $0.name == "selected-tip" }?.tip == base)
    #expect(try String(contentsOf: file, encoding: .utf8) == "Unsaved work")
    #expect(throws: (any Error).self) { try git.createBranch(named: "selected-tip", startingAt: "HEAD", in: root) }
    #expect(throws: (any Error).self) { try git.createBranch(named: "invalid name", startingAt: base, in: root) }
    #expect(throws: (any Error).self) { try git.createBranch(named: "missing", startingAt: "missing-target", in: root) }
    let selectedSource = GitBranch(name: "selected-tip", isCurrent: false, isRemote: false, tip: base, subject: "Base")
    try runGit(["branch", "--force", "selected-tip", "HEAD"], in: root)
    #expect(throws: (any Error).self) {
        try git.createBranch(named: "stale-source", startingAt: base, expectedSourceBranch: selectedSource, in: root)
    }
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "stale-source" } == nil)
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "selected-tip" }?.tip == before.headHash)
}

@Test func createBranchAtHeadRejectsChangedCheckout() throws {
    // Arrange
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let selected = try git.loadSnapshot(at: root)
    let selectedHead = try #require(selected.headHash)
    try runGit(["switch", "-c", "other"], in: root)

    // Act
    #expect(throws: (any Error).self) {
        try git.createBranch(named: "wrong-checkout", expectedBranch: selected.currentBranch, expectedHead: selectedHead, in: root)
    }
    try runGit(["switch", selected.currentBranch], in: root)
    try runGit(["commit", "--allow-empty", "-m", "New HEAD"], in: root)
    #expect(throws: (any Error).self) {
        try git.createBranch(named: "wrong-head", expectedBranch: selected.currentBranch, expectedHead: selectedHead, in: root)
    }
    let current = try git.loadSnapshot(at: root)
    try git.createBranch(named: "correct-head", expectedBranch: current.currentBranch, expectedHead: current.headHash, in: root)

    // Assert
    let after = try git.loadSnapshot(at: root)
    #expect(after.currentBranch == "correct-head")
    #expect(after.headHash == current.headHash)
    #expect(after.branches.allSatisfy { $0.name != "wrong-checkout" && $0.name != "wrong-head" })

    let unborn = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: unborn, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: unborn) }
    try git.initialize(at: unborn)
    let empty = try git.loadSnapshot(at: unborn)
    #expect(empty.headHash == nil)
    try git.createBranch(named: "first-branch", expectedBranch: empty.currentBranch, expectedHead: empty.headHash, in: unborn)
    #expect(try git.loadSnapshot(at: unborn).currentBranch == "first-branch")
}

@Test func repositoryRootPreservesTrailingWhitespace() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("repository \n")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: root)
    let snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.rootPath.hasSuffix("repository \n"))
    #expect(snapshot.name == "repository \n")
    #expect(snapshot.commits.isEmpty)
}

@Test func linkedWorktreeSnapshotsIdentifyEachCheckout() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let main = base.appendingPathComponent("main")
    let linked = base.appendingPathComponent("linked checkout")
    try FileManager.default.createDirectory(at: main, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: main)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: main)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: main)
    try runGit(["branch", "linked-feature"], in: main)
    let selectedTip = try #require(git.loadSnapshot(at: main).headHash)
    let missingDestination = base.appendingPathComponent("missing branch checkout")
    #expect(throws: (any Error).self) {
        try git.createWorktree(branch: "does-not-exist", at: missingDestination, in: main)
    }
    #expect(!FileManager.default.fileExists(atPath: missingDestination.path))
    let occupied = base.appendingPathComponent("occupied")
    try FileManager.default.createDirectory(at: occupied, withIntermediateDirectories: true)
    let existing = occupied.appendingPathComponent("keep.txt")
    try "Keep this content".write(to: existing, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) {
        try git.createWorktree(branch: "linked-feature", at: occupied, in: main)
    }
    #expect(try String(contentsOf: existing, encoding: .utf8) == "Keep this content")
    try runGit(["commit", "--allow-empty", "-m", "Move selected branch"], in: main)
    try runGit(["branch", "--force", "linked-feature", "HEAD"], in: main)
    #expect(throws: (any Error).self) {
        try git.createWorktree(branch: "linked-feature", expectedTip: selectedTip, at: linked, in: main)
    }
    #expect(!FileManager.default.fileExists(atPath: linked.path))
    let currentTip = try #require(git.loadSnapshot(at: main).headHash)
    try git.createWorktree(branch: "linked-feature", expectedTip: currentTip, at: linked, in: main)
    #expect(throws: (any Error).self) {
        try git.createWorktree(branch: "linked-feature", at: base.appendingPathComponent("duplicate"), in: main)
    }
    let snapshot = try git.loadSnapshot(at: linked)
    #expect(snapshot.currentBranch == "linked-feature")
    #expect(snapshot.worktrees.count == 2)
    #expect(snapshot.worktrees.contains { URL(fileURLWithPath: $0.path).resolvingSymlinksInPath().path == linked.resolvingSymlinksInPath().path && $0.branch == "linked-feature" })
    #expect(URL(fileURLWithPath: snapshot.rootPath).resolvingSymlinksInPath().path == linked.resolvingSymlinksInPath().path)
}

@Test func mergeInspectorShowsChangesAgainstFirstParent() throws {
    let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try runGit(["init", "--initial-branch=main"], in: root)
    let git = GitClient()
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["checkout", "-b", "feature"], in: root)
    try "Feature addition\n".write(to: root.appendingPathComponent("feature.txt"), atomically: true, encoding: .utf8)
    try runGit(["add", "."], in: root)
    try runGit(["commit", "-m", "Feature"], in: root)
    try runGit(["checkout", "main"], in: root)
    try "Main addition\n".write(to: root.appendingPathComponent("main.txt"), atomically: true, encoding: .utf8)
    try runGit(["add", "."], in: root)
    try runGit(["commit", "-m", "Main"], in: root)
    try runGit(["merge", "--no-ff", "feature", "-m", "Merge feature"], in: root)

    #expect(try git.commitFiles(hash: "HEAD", in: root) == ["feature.txt"])
    let patch = try git.commitDiff(hash: "HEAD", path: "feature.txt", in: root)
    #expect(patch.contains("+Feature addition"))
    #expect(!patch.contains("+Main addition"))
    #expect(try git.commitDiff(hash: "HEAD", in: root).contains("+Feature addition"))
    let mergeHash = try #require(git.loadSnapshot(at: root).headHash)
    try git.start(.revert, target: "HEAD", mainline: 1, in: root)
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("feature.txt").path))
    #expect(try String(contentsOf: root.appendingPathComponent("main.txt"), encoding: .utf8) == "Main addition\n")
    try git.start(.cherryPick, target: mergeHash, mainline: 1, in: root)
    #expect(try String(contentsOf: root.appendingPathComponent("feature.txt"), encoding: .utf8) == "Feature addition\n")
    #expect(try String(contentsOf: root.appendingPathComponent("main.txt"), encoding: .utf8) == "Main addition\n")
    #expect(try git.loadSnapshot(at: root).operation == nil)
}

@Test func gitClientLoadsSnapshotFromARealRepository() throws {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("NiceGitCoreTests-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer {
        try? FileManager.default.removeItem(at: root)
    }

    try runGit(["init", "--initial-branch=master"], in: root)
    try "NiceGit smoke repo\n".write(to: root.appendingPathComponent("README.md"), atomically: true, encoding: .utf8)
    try runGit(["add", "README.md"], in: root)
    try runGit([
        "-c",
        "user.name=NiceGit",
        "-c",
        "user.email=smoke@nicegit.local",
        "commit",
        "-m",
        "Initial smoke commit",
        "-m",
        "Detailed explanation.\nSecond body line."
    ], in: root)
    try runGit(["branch", "feature/free-client"], in: root)
    try "NiceGit smoke repo\nchanged\n".write(to: root.appendingPathComponent("README.md"), atomically: true, encoding: .utf8)

    let snapshot = try GitClient().loadSnapshot(at: root)

    #expect(snapshot.name == root.lastPathComponent)
    #expect(snapshot.currentBranch == "master")
    #expect(snapshot.commits.map(\.subject) == ["Initial smoke commit"])
    #expect(try GitClient().commitMessage(hash: snapshot.commits[0].hash, in: root).contains("Detailed explanation.\nSecond body line."))
    #expect(snapshot.branches.contains { $0.name == "feature/free-client" })
    #expect(snapshot.status.count == 1)
    #expect(snapshot.status[0].path == "README.md")
    #expect(snapshot.status[0].kind == .modified)
    #expect(snapshot.headHash == snapshot.commits.first?.hash)
    #expect(try GitClient().commitFiles(hash: snapshot.commits[0].hash, in: root) == ["README.md"])
    #expect(try GitClient().commitDiff(hash: snapshot.commits[0].hash, path: "README.md", in: root).contains("+NiceGit smoke repo"))

    let git = GitClient()
    let workingDiff = try git.diff(path: "README.md", staged: false, in: root)
    #expect(workingDiff.contains("+changed"))
    try git.stage(path: "README.md", in: root)
    #expect(try git.diff(path: "README.md", staged: true, in: root).contains("+changed"))
    #expect(try git.diff(path: "README.md", staged: false, in: root).isEmpty)
    #expect(try git.commitDiff(hash: snapshot.commits[0].hash, in: root).contains("+NiceGit smoke repo"))

    let largeContent = (0..<10000).map { "Added line \($0)" }.joined(separator: "\n")
    try largeContent.write(to: root.appendingPathComponent("new file.txt"), atomically: true, encoding: .utf8)
    let newDiff = try git.diff(path: "new file.txt", staged: false, untracked: true, in: root)
    #expect(newDiff.contains("+Added line 9999"))
    #expect(newDiff.utf8.count > 100000)

    try git.saveStash(message: "In progress", includeUntracked: true, in: root)
    let stashed = try git.loadSnapshot(at: root)
    #expect(stashed.status.isEmpty)
    #expect(stashed.commits.map(\.hash) == snapshot.commits.map(\.hash))
    let stash = try #require(stashed.stashes.first)
    #expect(stash.message.contains("In progress"))
    let stashPatch = try git.stashDiff(hash: stash.hash, in: root)
    #expect(stashPatch.contains("+changed"))
    #expect(stashPatch.contains("+Added line 9999"))
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("new file.txt").path))
    try git.applyStash(stash, in: root)
    #expect(try git.loadSnapshot(at: root).stagedCount == 1)
    #expect(try String(contentsOf: root.appendingPathComponent("new file.txt"), encoding: .utf8) == largeContent)
    #expect(try git.listStashes(in: root).count == 1)
    try git.dropStash(stash, in: root)
    #expect(try git.listStashes(in: root).isEmpty)

    let destination = root.appendingPathComponent("clone")
    try git.clone(source: root.path, to: destination)
    let cloned = try git.loadSnapshot(at: destination)
    #expect(cloned.commits.first?.hash == snapshot.commits.first?.hash)
    #expect(cloned.status.isEmpty)
    #expect(cloned.upstream == "origin/master")
    #expect(cloned.ahead == 0)
    #expect(cloned.behind == 0)
    #expect(cloned.branches.contains { $0.isRemote && $0.displayName == "origin/master" })
    try git.checkoutRemote(branch: "remotes/origin/feature/free-client", in: destination)
    #expect(try git.loadSnapshot(at: destination).currentBranch == "feature/free-client")
    try git.createBranch(named: "published-from-nicegit", in: destination)
    try git.publish(remote: "origin", in: destination)
    #expect(try git.loadSnapshot(at: root).branches.contains { $0.name == "published-from-nicegit" && !$0.isRemote })
    try git.push(in: destination)
    try git.setIdentity(name: "Test", email: "test@example.com", in: destination)
    try runGit(["commit", "--allow-empty", "-m", "Ahead of remote"], in: destination)
    #expect(try git.loadSnapshot(at: destination).ahead == 1)
    try git.push(in: destination)
    #expect(try git.loadSnapshot(at: destination).ahead == 0)

    for name in ["space name.txt", "arrow -> name.txt", "line\nbreak.txt", "[literal].txt"] {
        try "content".write(to: destination.appendingPathComponent(name), atomically: true, encoding: .utf8)
        #expect(try git.loadSnapshot(at: destination).status.contains { $0.path == name })
        try git.stage(path: name, in: destination)
        #expect(try git.loadSnapshot(at: destination).status.contains { $0.path == name && $0.isStaged })
    }
}

@Test(arguments: [false, true]) func unstageBeforeFirstCommitPreservesFiles(unstageAll: Bool) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try runGit(["init"], in: root)
    let file = root.appendingPathComponent("first.txt")
    let other = root.appendingPathComponent("other.txt")
    try "staged version\n".write(to: file, atomically: true, encoding: .utf8)
    try "other file\n".write(to: other, atomically: true, encoding: .utf8)
    let git = GitClient()
    try git.stageAll(in: root)
    try "newer working version\n".write(to: file, atomically: true, encoding: .utf8)

    if unstageAll { try git.unstageAll(in: root) }
    else { try git.unstage(path: "first.txt", in: root) }

    let status = try git.loadStatus(in: root)
    #expect(status.first { $0.path == "first.txt" }?.kind == .untracked)
    #expect(status.first { $0.path == "other.txt" }?.isStaged == !unstageAll)
    #expect(try String(contentsOf: file, encoding: .utf8) == "newer working version\n")
    #expect(try String(contentsOf: other, encoding: .utf8) == "other file\n")
}

@Test func mergeConflictCanContinueAndRebaseCanAbort() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try GitClient().initialize(at: root)
    try GitClient().setIdentity(name: "Test", email: "test@example.com", in: root)
    #expect(GitClient().identity(in: root).name == "Test")
    #expect(GitClient().identity(in: root).email == "test@example.com")
    try GitClient().addRemote(name: "upstream", address: root.appendingPathComponent("remote.git").path, in: root)
    #expect(try GitClient().loadSnapshot(at: root).remotes == ["upstream"])
    let file = root.appendingPathComponent("file.txt")
    let git = GitClient()
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", in: root)
    try "feature\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Feature", in: root)
    let featureHash = try #require(git.loadSnapshot(at: root).commits.first?.hash)
    try git.checkout(branch: "main", in: root)
    try "main\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Main", in: root)
    try "saved\n".write(to: root.appendingPathComponent("saved.txt"), atomically: true, encoding: .utf8)
    try git.saveStash(message: "Before integration", includeUntracked: true, in: root)
    let savedStash = try #require(git.listStashes(in: root).first)

    #expect(throws: (any Error).self) { try git.start(.rebase, target: "feature", in: root) }
    #expect(try git.loadSnapshot(at: root).operation == .rebase)
    try git.abortOperation(.rebase, in: root)
    #expect(try git.loadSnapshot(at: root).operation == nil)
    #expect(try String(contentsOf: file, encoding: .utf8) == "main\n")

    #expect(throws: (any Error).self) { try git.start(.cherryPick, target: featureHash, in: root) }
    #expect(try git.loadSnapshot(at: root).operation == .cherryPick)
    try git.abortOperation(.cherryPick, in: root)

    #expect(throws: (any Error).self) { try git.start(.merge, target: "feature", in: root) }
    let conflicted = try git.loadSnapshot(at: root)
    #expect(conflicted.operation == .merge)
    #expect(conflicted.status.first?.kind == .conflicted)
    let document = try git.loadConflict(path: "file.txt", in: root)
    #expect(document.content.contains("<<<<<<<"))
    #expect(document.base == "base\n")
    #expect(document.current == "main\n")
    #expect(document.incoming == "feature\n")
    #expect(throws: (any Error).self) { try git.resolveConflict(document, content: document.content, in: root) }
    try "external edit\n".write(to: file, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.resolveConflict(document, content: "stale edit\n", in: root) }
    #expect(try String(contentsOf: file, encoding: .utf8) == "external edit\n")
    let reloaded = try git.loadConflict(path: "file.txt", in: root)
    try git.resolveConflict(reloaded, content: "resolved\n", in: root)
    let readyToContinue = try git.loadSnapshot(at: root)
    #expect(readyToContinue.operation == .merge)
    do {
        try git.checkout(branch: "feature", in: root)
        Issue.record("Branch checkout ran during an unfinished merge")
    } catch {
        #expect(error.localizedDescription.contains("Finish or abort"))
    }
    do {
        try git.createBranch(named: "wrong-merge-branch", in: root)
        Issue.record("Branch creation changed checkout during an unfinished merge")
    } catch {
        #expect(error.localizedDescription.contains("Finish or abort"))
    }
    for action in [
        { try git.saveStash(message: "Wrong time", includeUntracked: true, in: root) },
        { try git.applyStash(savedStash, in: root) },
        { try git.popStash(savedStash, in: root) }
    ] {
        do {
            try action()
            Issue.record("A stash action ran during an unfinished merge")
        } catch {
            #expect(error.localizedDescription.contains("Finish or abort"))
        }
    }
    let stillMerging = try git.loadSnapshot(at: root)
    #expect(stillMerging.operation == .merge)
    #expect(stillMerging.currentBranch == "main")
    #expect(stillMerging.stashes == readyToContinue.stashes)
    #expect(stillMerging.status == readyToContinue.status)
    #expect(stillMerging.branches.allSatisfy { $0.name != "wrong-merge-branch" })
    #expect(try String(contentsOf: file, encoding: .utf8) == "resolved\n")
    try git.continueOperation(.merge, in: root)
    let merged = try git.loadSnapshot(at: root)
    #expect(merged.operation == nil)
    #expect(merged.status.isEmpty)
    #expect(merged.commits.first?.parents.count == 2)
    #expect(try String(contentsOf: file, encoding: .utf8) == "resolved\n")
    let firstPage = try git.loadSnapshot(at: root, historyLimit: 1)
    #expect(firstPage.commits.count == 1)
    #expect(firstPage.hasMoreCommits)
    let fullPage = try git.loadSnapshot(at: root, historyLimit: merged.commits.count)
    #expect(fullPage.commits.map(\.hash) == merged.commits.map(\.hash))
    #expect(!fullPage.hasMoreCommits)
}

@Test func renameUnstagingAndDetachedHistoryPreserveCompleteChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.com", in: root)
    try "content\n".write(to: root.appendingPathComponent("old.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try runGit(["mv", "old.txt", "new.txt"], in: root)
    let entry = try #require(git.loadSnapshot(at: root).status.first)
    #expect(entry.originalPath == "old.txt")
    let diff = try git.diff(path: entry.path, staged: true, originalPath: entry.originalPath, in: root)
    #expect(diff.contains("rename from old.txt"))
    #expect(diff.contains("rename to new.txt"))
    try git.unstage(path: entry.path, originalPath: entry.originalPath, in: root)
    #expect(try git.loadSnapshot(at: root).stagedCount == 0)
    #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent("new.txt").path))
    try runGit(["switch", "--detach"], in: root)
    try git.stageAll(in: root)
    try git.commit(message: "Detached commit", in: root)
    let detached = try git.loadSnapshot(at: root)
    #expect(detached.currentBranch.hasPrefix("Detached HEAD"))
    #expect(detached.headHash == detached.commits.first?.hash)
    #expect(detached.commits.first?.subject == "Detached commit")
}

@Test func snapshotShowsLocalUpstreamAndDivergence() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try runGit(["switch", "-c", "feature"], in: root)
    try runGit(["branch", "--set-upstream-to=main", "feature"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Feature work"], in: root)
    try runGit(["switch", "main"], in: root)
    try runGit(["commit", "--allow-empty", "-m", "Main work"], in: root)
    try runGit(["switch", "feature"], in: root)

    let snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.currentBranch == "feature")
    #expect(snapshot.headHash == snapshot.branches.first { $0.isCurrent }?.tip)
    #expect(snapshot.upstream == "main")
    #expect(snapshot.ahead == 1)
    #expect(snapshot.behind == 1)
}

@Test func pagedHistoryKeepsOlderCurrentHeadVisible() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    let head = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["switch", "-c", "busy"], in: root)
    for number in 0..<8 {
        try runGit(["commit", "--allow-empty", "-m", "Busy \(number)"], in: root)
    }
    try runGit(["switch", "main"], in: root)

    let firstPage = try git.loadSnapshot(at: root, historyLimit: 3)
    #expect(firstPage.currentBranch == "main")
    #expect(firstPage.headHash == head)
    #expect(firstPage.commits.count == 4)
    #expect(firstPage.commits.last?.hash == head)
    #expect(firstPage.hasMoreCommits)
    #expect(GitGraph.layoutWithWorkingTree(firstPage.commits, headHash: head).count == 5)

    let fullHistory = try git.loadSnapshot(at: root, historyLimit: 20)
    #expect(fullHistory.commits.count == 9)
    #expect(fullHistory.commits.filter { $0.hash == head }.count == 1)
    #expect(!fullHistory.hasMoreCommits)
}

@Test func branchManagementPreservesUnmergedWork() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.com", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Base"], in: root)
    try git.createBranch(named: "feature", in: root)
    try runGit(["commit", "--allow-empty", "-m", "Unmerged work"], in: root)
    try git.renameBranch("feature", to: "renamed", in: root)
    #expect(try git.loadSnapshot(at: root).currentBranch == "renamed")
    #expect(throws: (any Error).self) { try git.deleteBranch("renamed", in: root) }
    try git.checkout(branch: "main", in: root)
    #expect(throws: (any Error).self) { try git.deleteBranch("renamed", in: root) }
    #expect(try git.loadSnapshot(at: root).branches.contains { $0.name == "renamed" })
    try git.start(.merge, target: "renamed", in: root)
    try git.deleteBranch("renamed", in: root)
    #expect(try git.loadSnapshot(at: root).branches.allSatisfy { $0.name != "renamed" })
    try git.createTag(name: "v1.0.0", target: "HEAD", in: root)
    #expect(try git.loadSnapshot(at: root).tags == ["v1.0.0"])
    #expect(try git.commitDiff(hash: "refs/tags/v1.0.0", in: root).contains("Unmerged work"))
    #expect(throws: (any Error).self) { try git.createTag(name: "v1.0.0", target: "HEAD", in: root) }
    #expect(throws: (any Error).self) { try git.createTag(name: "invalid tag", target: "HEAD", in: root) }
    try git.deleteTag(name: "v1.0.0", in: root)
    #expect(try git.loadSnapshot(at: root).tags.isEmpty)
}

@Test func binaryConflictCanChooseIncomingOrDeletion() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.com", in: root)
    let file = root.appendingPathComponent("binary.dat")
    try Data([0, 1, 2]).write(to: file)
    try git.stageAll(in: root)
    try git.commit(message: "Base binary", in: root)
    try git.createBranch(named: "incoming", in: root)
    try Data([0, 3, 4, 255]).write(to: file)
    try git.stageAll(in: root)
    try git.commit(message: "Incoming binary", in: root)
    try git.checkout(branch: "main", in: root)
    try Data([0, 5, 6]).write(to: file)
    try git.stageAll(in: root)
    try git.commit(message: "Current binary", in: root)
    #expect(throws: (any Error).self) { try git.start(.merge, target: "incoming", in: root) }
    #expect(throws: (any Error).self) { try git.loadConflict(path: "binary.dat", in: root) }
    try git.resolveConflictSide(path: "binary.dat", incoming: true, in: root)
    #expect(try Data(contentsOf: file) == Data([0, 3, 4, 255]))
    #expect(try git.loadSnapshot(at: root).status.allSatisfy { $0.kind != .conflicted })
    try git.abortOperation(.merge, in: root)
    #expect(throws: (any Error).self) { try git.start(.merge, target: "incoming", in: root) }
    try git.resolveConflictDeletion(path: "binary.dat", in: root)
    #expect(!FileManager.default.fileExists(atPath: file.path))
    #expect(try git.loadSnapshot(at: root).status.first?.kind == .deleted)
    try git.continueOperation(.merge, in: root)
    #expect(try git.loadSnapshot(at: root).operation == nil)
}

private func runGitOutput(_ arguments: [String], in directory: URL) throws -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["git"] + arguments
    process.currentDirectoryURL = directory
    process.environment = ["PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"]
    let output = Pipe()
    process.standardOutput = output
    try process.run()
    let data = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw GitClientError.commandFailed(command: arguments.joined(separator: " "), message: "exit \(process.terminationStatus)")
    }
    return String(decoding: data, as: UTF8.self)
}

private func runGit(_ arguments: [String], in directory: URL, environment: [String: String] = [:]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["git"] + arguments
    process.currentDirectoryURL = directory
    process.environment = ["PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"].merging(environment) { _, new in new }

    let error = Pipe()
    process.standardError = error
    try process.run()
    process.waitUntilExit()

    guard process.terminationStatus == 0 else {
        let data = error.fileHandleForReading.readDataToEndOfFile()
        let message = String(data: data, encoding: .utf8) ?? "Git failed."
        throw NSError(domain: "NiceGitCoreTests", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: message])
    }
}

@Test func pullPreservesIgnoredLocalFiles() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let remote = base.appendingPathComponent("remote.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try runGit(["init", "--bare"], in: remote)
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "local.txt\n".write(to: root.appendingPathComponent(".gitignore"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.addRemote(name: "origin", address: remote.path, in: root)
    try runGit(["push", "--set-upstream", "origin", "main"], in: root)
    let original = try #require(git.loadSnapshot(at: root).headHash)
    let file = root.appendingPathComponent("local.txt")
    try "remote contents\n".write(to: file, atomically: true, encoding: .utf8)
    try runGit(["add", "--force", "local.txt"], in: root)
    try git.commit(message: "Track local file", in: root)
    try runGit(["push", "origin", "main"], in: root)
    let incoming = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["reset", "--hard", original], in: root)
    try "local contents\n".write(to: file, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.pull(in: root) }
    #expect(try git.loadSnapshot(at: root).headHash == original)
    #expect(try String(contentsOf: file, encoding: .utf8) == "local contents\n")
    try FileManager.default.removeItem(at: file)
    try git.pull(in: root)
    #expect(try git.loadSnapshot(at: root).headHash == incoming)
    #expect(try String(contentsOf: file, encoding: .utf8) == "remote contents\n")
}

@Test(arguments: [false, true]) func pullUpdatesInitializedSubmodulesWhenConfigured(recurse: Bool) throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let source = base.appendingPathComponent("source")
    let remote = base.appendingPathComponent("remote")
    let checkout = base.appendingPathComponent("checkout")
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    for url in [source, remote] {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try git.initialize(at: url)
        try git.setIdentity(name: "Test", email: "test@example.invalid", in: url)
    }
    let file = source.appendingPathComponent("file.txt")
    try "one\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: source)
    try git.commit(message: "One", in: source)
    try runGit(["-c", "protocol.file.allow=always", "submodule", "add", source.path, "module"], in: remote)
    try git.commit(message: "Base", in: remote)
    try runGit(["-c", "protocol.file.allow=always", "clone", "--recurse-submodules", remote.path, checkout.path], in: base)
    try runGit(["config", "submodule.recurse", recurse ? "true" : "false"], in: checkout)
    try "two\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: source)
    try git.commit(message: "Two", in: source)
    let tip = try #require(git.loadSnapshot(at: source).headHash)
    let remoteModule = remote.appendingPathComponent("module")
    try runGit(["fetch"], in: remoteModule)
    try runGit(["checkout", tip], in: remoteModule)
    try git.stageAll(in: remote)
    try git.commit(message: "Advance submodule", in: remote)
    // Pre-fetch the local fixture's objects; recursive fetch deliberately restricts file transport.
    try runGit(["fetch"], in: checkout.appendingPathComponent("module"))
    try git.pull(in: checkout)
    #expect(try String(contentsOf: checkout.appendingPathComponent("module/file.txt"), encoding: .utf8) == (recurse ? "two\n" : "one\n"))
    #expect(try git.loadStatus(in: checkout).isEmpty == recurse)
}

@Test(arguments: ["pull-on", "pull-off", "rebase-on", "branch-off"])
func pullRespectsAutoStashConfiguration(setting: String) throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let remote = base.appendingPathComponent("remote")
    let checkout = base.appendingPathComponent("checkout")
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: remote)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: remote)
    let middle = String(repeating: "unchanged\n", count: 20)
    let file = remote.appendingPathComponent("file.txt")
    try ("first\n" + middle + "last\n").write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: remote)
    try git.commit(message: "Base", in: remote)
    try git.clone(source: remote.path, to: checkout)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: checkout)
    let original = try #require(git.loadSnapshot(at: checkout).headHash)
    try runGit(["config", "merge.autostash", setting == "pull-off" ? "true" : "false"], in: checkout)
    if setting.hasPrefix("pull-") {
        try runGit(["config", "pull.autostash", setting == "pull-on" ? "true" : "false"], in: checkout)
    } else {
        try runGit(["config", "pull.rebase", "merges"], in: checkout)
        try runGit(["config", "rebase.autostash", "true"], in: checkout)
        if setting == "branch-off" { try runGit(["config", "branch.main.rebase", "false"], in: checkout) }
    }
    let working = checkout.appendingPathComponent("file.txt")
    let local = "local\n" + middle + "last\n"
    try local.write(to: working, atomically: true, encoding: .utf8)
    try ("first\n" + middle + "remote\n").write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: remote)
    try git.commit(message: "Remote", in: remote)
    if setting.hasSuffix("-on") {
        try git.pull(in: checkout)
        #expect(try String(contentsOf: working, encoding: .utf8) == "local\n" + middle + "remote\n")
        #expect(try git.loadSnapshot(at: checkout).headHash != original)
    } else {
        #expect(throws: (any Error).self) { try git.pull(in: checkout) }
        #expect(try String(contentsOf: working, encoding: .utf8) == local)
        #expect(try git.loadSnapshot(at: checkout).headHash == original)
    }
}

@Test func pullRejectsCheckoutChangedDuringFetch() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let remote = base.appendingPathComponent("remote")
    let checkout = base.appendingPathComponent("checkout")
    try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: remote)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: remote)
    try "base\n".write(to: remote.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: remote)
    try git.commit(message: "Base", in: remote)
    try git.clone(source: remote.path, to: checkout)
    try runGit(["branch", "other"], in: checkout)
    let original = try #require(git.loadSnapshot(at: checkout).headHash)
    try "remote\n".write(to: remote.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: remote)
    try git.commit(message: "Remote", in: remote)
    let hook = base.appendingPathComponent("upload-pack.sh")
    let quotedCheckout = "'" + checkout.path.replacingOccurrences(of: "'", with: "'\\''") + "'"
    try ("#!/bin/sh\n/usr/bin/env -u GIT_DIR -u GIT_WORK_TREE /usr/bin/git -C " + quotedCheckout + " switch other >&2\nexec /usr/bin/git upload-pack \"$@\"\n")
        .write(to: hook, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: hook.path)
    try runGit(["config", "remote.origin.uploadpack", hook.path], in: checkout)
    #expect(throws: (any Error).self) { try git.pull(expectedBranch: "main", expectedHead: original, in: checkout) }
    let after = try git.loadSnapshot(at: checkout)
    #expect(after.currentBranch == "other")
    #expect(after.headHash == original)
    #expect(try String(contentsOf: checkout.appendingPathComponent("file.txt"), encoding: .utf8) == "base\n")
}
@Test(arguments: [3, 9]) func conflictEditorHonorsCustomMarkerSize(markerSize: Int) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try runGit(["config", "merge.conflictStyle", "diff3"], in: root)
    let file = root.appendingPathComponent("file.txt")
    try "file.txt conflict-marker-size=\(markerSize)\n".write(to: root.appendingPathComponent(".gitattributes"), atomically: true, encoding: .utf8)
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", in: root)
    try "feature\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Feature", in: root)
    try git.checkout(branch: "main", in: root)
    try "main\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Main", in: root)
    #expect(throws: (any Error).self) { try git.start(.merge, target: "feature", in: root) }
    let document = try git.loadConflict(path: "file.txt", in: root)
    #expect(document.content.contains(String(repeating: "<", count: markerSize) + " HEAD"))
    #expect(throws: (any Error).self) { try git.resolveConflict(document, content: document.content, in: root) }
    #expect(try Data(contentsOf: file) == document.originalData)
    #expect(try git.loadStatus(in: root).contains { $0.path == "file.txt" && $0.kind == .conflicted })
    if markerSize == 9 {
        let resolved = "Heading\n=======\nresolved\n"
        try git.resolveConflict(document, content: resolved, in: root)
        #expect(try String(contentsOf: file, encoding: .utf8) == resolved)
        #expect(try git.loadStatus(in: root).allSatisfy { $0.kind != .conflicted })
    }
}

@Test func restoreFileFromCommitReplacesStagedAndUnstagedEditsOfThatPathOnly() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    let glob = root.appendingPathComponent("*.txt")
    let other = root.appendingPathComponent("other.txt")
    for url in [file, glob, other] { try "one\n".write(to: url, atomically: true, encoding: .utf8) }
    try git.stageAll(in: root)
    try git.commit(message: "One", in: root)
    let first = try #require(git.loadSnapshot(at: root).headHash)
    for url in [file, glob, other] { try "two\n".write(to: url, atomically: true, encoding: .utf8) }
    try git.stageAll(in: root)
    try git.commit(message: "Two", in: root)
    try "staged\n".write(to: glob, atomically: true, encoding: .utf8)
    try git.stage(path: "*.txt", in: root)
    try "unstaged\n".write(to: glob, atomically: true, encoding: .utf8)
    try "local\n".write(to: other, atomically: true, encoding: .utf8)
    let snapshot = try git.loadSnapshot(at: root)

    // A glob-like name restores only that literal path.
    try git.restore(path: "*.txt", from: first, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)

    #expect(try String(contentsOf: glob, encoding: .utf8) == "one\n")
    #expect(try String(contentsOf: file, encoding: .utf8) == "two\n")
    #expect(try String(contentsOf: other, encoding: .utf8) == "local\n")
    let status = try git.loadStatus(in: root)
    #expect(status.first { $0.path == "*.txt" }?.indexStatus == "M")
    #expect(status.first { $0.path == "*.txt" }?.workTreeStatus == " ")
    #expect(status.first { $0.path == "other.txt" }?.workTreeStatus == "M")
    #expect(try git.loadSnapshot(at: root).headHash == snapshot.headHash)
}

@Test func restoreFileFromCommitRecreatesAndRemovesFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let removed = root.appendingPathComponent("folder/removed.txt")
    try FileManager.default.createDirectory(at: removed.deletingLastPathComponent(), withIntermediateDirectories: true)
    try "kept in history\n".write(to: removed, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add", in: root)
    let first = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["rm", "-r", "--quiet", "folder"], in: root)
    try "new\n".write(to: root.appendingPathComponent("added.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Remove and add", in: root)
    var snapshot = try git.loadSnapshot(at: root)

    try git.restore(path: "folder/removed.txt", from: first, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)
    #expect(try String(contentsOf: removed, encoding: .utf8) == "kept in history\n")
    #expect(try git.loadStatus(in: root).contains { $0.path == "folder/removed.txt" && $0.indexStatus == "A" })

    snapshot = try git.loadSnapshot(at: root)
    try git.restore(path: "added.txt", from: first, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("added.txt").path))
    #expect(try git.loadStatus(in: root).contains { $0.path == "added.txt" && $0.indexStatus == "D" })
}

@Test func restoreFileFromCommitRefusesUntrackedFilesStaleCheckoutsAndUnfinishedOperations() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("file.txt")
    let later = root.appendingPathComponent("later.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try "later\n".write(to: later, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    let base = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["rm", "--quiet", "later.txt"], in: root)
    try git.commit(message: "Remove later", in: root)

    // An untracked local file at the restored path is never replaced.
    try "mine\n".write(to: later, atomically: true, encoding: .utf8)
    var snapshot = try git.loadSnapshot(at: root)
    #expect(throws: (any Error).self) {
        try git.restore(path: "later.txt", from: base, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)
    }
    #expect(try String(contentsOf: later, encoding: .utf8) == "mine\n")
    try FileManager.default.removeItem(at: later)

    // A dialog opened before HEAD moved cannot act on the new checkout.
    try "edit\n".write(to: file, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) {
        try git.restore(path: "file.txt", from: base, expectedBranch: snapshot.currentBranch, expectedHead: base, in: root)
    }
    #expect(try String(contentsOf: file, encoding: .utf8) == "edit\n")
    try runGit(["checkout", "--quiet", "--", "file.txt"], in: root)

    // Files are left alone while a merge is unfinished.
    try git.createBranch(named: "feature", in: root)
    try "feature\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Feature", in: root)
    try git.checkout(branch: "main", in: root)
    try "main\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Main", in: root)
    #expect(throws: (any Error).self) { try git.start(.merge, target: "feature", in: root) }
    snapshot = try git.loadSnapshot(at: root)
    let conflicted = try Data(contentsOf: file)
    #expect(throws: (any Error).self) {
        try git.restore(path: "file.txt", from: base, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)
    }
    #expect(try Data(contentsOf: file) == conflicted)
}

@Test func fileHistoryFollowsRenamesAndKeepsUnusualNamesLiteral() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    #expect(try git.fileHistory(path: "missing.txt", in: root).isEmpty)
    let original = "\nfirst\u{1e}name.txt"
    try "one\ntwo\nthree\nfour\n".write(to: root.appendingPathComponent(original), atomically: true, encoding: .utf8)
    try "other\n".write(to: root.appendingPathComponent("*.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add", in: root)
    try "changed\n".write(to: root.appendingPathComponent("*.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Only the glob-named file", in: root)
    try runGit(["mv", original, "renamed.txt"], in: root)
    try git.commit(message: "Rename", in: root)
    try "one\ntwo\nthree\nfour\nfive\n".write(to: root.appendingPathComponent("renamed.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Edit", in: root)

    let history = try git.fileHistory(path: "renamed.txt", in: root)
    #expect(history.map(\.commit.subject) == ["Edit", "Rename", "Add"])
    #expect(history.map(\.path) == ["renamed.txt", "renamed.txt", original])
    #expect(history.map(\.status) == ["M", "R", "A"])
    #expect(try git.fileHistory(path: "*.txt", in: root).map(\.commit.subject) == ["Only the glob-named file", "Add"])
    #expect(try git.fileHistory(path: "renamed.txt", limit: 1, in: root).count == 1)
}

@Test func fileHistoryMarksTheCommitThatDeletedAFile() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("gone.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add", in: root)
    try runGit(["rm", "--quiet", "gone.txt"], in: root)
    try git.commit(message: "Delete", in: root)
    let history = try git.fileHistory(path: "gone.txt", in: root)
    #expect(history.map(\.commit.subject) == ["Delete", "Add"])
    #expect(history.map(\.deletesFile) == [true, false])
}

@Test func blameAttributesLinesToCommitsAndUncommittedEdits() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "First Author", email: "first@example.invalid", in: root)
    let file = root.appendingPathComponent("*.txt")
    try "one\r\ntwo\r\nthree\r\n".write(to: file, atomically: true, encoding: .utf8)
    try "other\n".write(to: root.appendingPathComponent("a.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add lines", in: root)
    let first = try #require(git.loadSnapshot(at: root).headHash)
    try git.setIdentity(name: "Second Author", email: "second@example.invalid", in: root)
    try "one\r\nTWO\r\nthree\r\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Shout the second line", in: root)
    try "one\r\nTWO\r\nthree\r\nfour\r\n".write(to: file, atomically: true, encoding: .utf8)

    let lines = try git.blame(path: "*.txt", in: root)
    #expect(lines.map(\.number) == [1, 2, 3, 4])
    #expect(lines.map(\.content) == ["one\r", "TWO\r", "three\r", "four\r"])
    #expect(lines.map(\.commit.authorName) == ["First Author", "Second Author", "First Author", "Not Committed Yet"])
    #expect(lines[1].commit.summary == "Shout the second line")
    #expect(lines[0].commit.hash == first && lines[0].commit.authorEmail == "first@example.invalid")
    #expect(lines[3].commit.isUncommitted && !lines[0].commit.isUncommitted)
    #expect(lines[0].commit.date != nil && lines[0].commit.path == "*.txt")

    let old = try git.blame(path: "*.txt", revision: first, in: root)
    #expect(old.map(\.content) == ["one\r", "two\r", "three\r"])
    #expect(Set(old.map(\.commit.hash)) == [first])

    try Data([0, 1, 2, 0]).write(to: root.appendingPathComponent("binary.bin"))
    try git.stage(path: "binary.bin", in: root)
    try git.commit(message: "Binary", in: root)
    #expect(throws: (any Error).self) { try git.blame(path: "binary.bin", in: root) }
}

@Test func blameParserUnquotesPaths() {
    #expect(GitBlameParser.unquote("\"tab\\there\\303\\251\\\\\"") == "tab\there\u{e9}\\")
    #expect(GitBlameParser.unquote("plain.txt") == "plain.txt")
}

@Test func remotesCanBeRenamedRepointedAndRemovedOnlyWhenUnchanged() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let server = base.appendingPathComponent("server.git")
    let other = base.appendingPathComponent("other.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    try runGit(["init", "--quiet", "--bare", server.path], in: base)
    try runGit(["init", "--quiet", "--bare", other.path], in: base)
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "One", in: root)
    try git.addRemote(name: "origin", address: server.path, in: root)
    try runGit(["push", "--quiet", "--set-upstream", "origin", "main"], in: root)

    // A dialog shown for another address cannot rename the remote.
    #expect(throws: (any Error).self) { try git.renameRemote("origin", to: "upstream", expectedAddress: other.path, in: root) }
    try git.renameRemote("origin", to: "upstream", expectedAddress: server.path, in: root)
    var snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.remotes == ["upstream"])
    #expect(snapshot.branches.contains { $0.isRemote && $0.displayName == "upstream/main" })
    #expect(snapshot.branches.first { $0.isCurrent }?.upstream == "refs/remotes/upstream/main")

    #expect(throws: (any Error).self) { try git.setRemoteAddress("upstream", to: "   ", expectedAddress: server.path, in: root) }
    try git.setRemoteAddress("upstream", to: other.path, expectedAddress: server.path, in: root)
    #expect(try git.remoteAddress(name: "upstream", in: root) == other.path)

    #expect(throws: (any Error).self) { try git.removeRemote("upstream", expectedAddress: server.path, in: root) }
    try git.removeRemote("upstream", expectedAddress: other.path, in: root)
    snapshot = try git.loadSnapshot(at: root)
    #expect(snapshot.remotes.isEmpty)
    #expect(!snapshot.branches.contains { $0.isRemote })
    #expect(throws: (any Error).self) { try git.removeRemote("upstream", expectedAddress: other.path, in: root) }
}

@Test func tagsArePushedAndDeletedOnRemotesOnlyWhenTheyMatch() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let server = base.appendingPathComponent("server.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    try runGit(["init", "--quiet", "--bare", server.path], in: base)
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "One", in: root)
    try git.addRemote(name: "origin", address: server.path, in: root)
    try git.createTag(name: "v1", target: "HEAD", message: "Release one", in: root)
    try git.createTag(name: "light", target: "HEAD", in: root)
    try git.createTag(name: "extra", target: "HEAD", in: root)
    var snapshot = try git.loadSnapshot(at: root)
    let addresses = snapshot.remotePushAddresses
    let serverTags = { try runGitOutput(["tag", "--list"], in: server).split(separator: "\n").map(String.init) }

    #expect(throws: (any Error).self) {
        try git.pushTag("v1", to: "origin", expectedTip: "0000000000000000000000000000000000000000", expectedPushAddresses: addresses, in: root)
    }
    #expect(throws: (any Error).self) {
        try git.pushTag("v1", to: "origin", expectedTip: snapshot.tagTips["v1"]!, expectedPushAddresses: ["origin": ["/elsewhere.git"]], in: root)
    }
    try git.pushTag("v1", to: "origin", expectedTip: snapshot.tagTips["v1"]!, expectedPushAddresses: addresses, in: root)
    try git.pushTag("light", to: "origin", expectedTip: snapshot.tagTips["light"]!, expectedPushAddresses: addresses, in: root)
    // Only the selected tags reach the remote; no branches or other tags follow.
    #expect(try serverTags() == ["light", "v1"])
    #expect(try runGitOutput(["for-each-ref", "refs/heads"], in: server).isEmpty)

    // A local tag moved to another commit is not pushed over the remote one or used to delete it.
    try "two\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Two", in: root)
    try runGit(["tag", "--force", "light", "HEAD"], in: root)
    snapshot = try git.loadSnapshot(at: root)
    #expect(throws: (any Error).self) {
        try git.pushTag("light", to: "origin", expectedTip: snapshot.tagTips["light"]!, expectedPushAddresses: addresses, in: root)
    }
    #expect(throws: (any Error).self) {
        try git.deleteRemoteTag("light", from: "origin", expectedTip: snapshot.tagTips["light"]!, expectedPushAddresses: addresses, in: root)
    }
    #expect(try serverTags() == ["light", "v1"])

    try git.deleteRemoteTag("v1", from: "origin", expectedTip: snapshot.tagTips["v1"]!, expectedPushAddresses: addresses, in: root)
    #expect(try serverTags() == ["light"])
    #expect(try git.loadSnapshot(at: root).tags.contains("v1"))
}

@Test func ignorePatternsMatchSelectedNamesLiterally() {
    #expect(GitClient.ignorePattern(for: "*.txt", rule: .path) == "/\\*.txt")
    #expect(GitClient.ignorePattern(for: "dir/[x]?.log", rule: .path) == "/dir/\\[x]\\?.log")
    #expect(GitClient.ignorePattern(for: "#note", rule: .path) == "/#note")
    #expect(GitClient.ignorePattern(for: "space  ", rule: .path) == "/space\\ \\ ")
    #expect(GitClient.ignorePattern(for: "build/", rule: .path) == "/build/")
    #expect(GitClient.ignorePattern(for: "logs/app.lo*", rule: .fileExtension) == "*.lo\\*")
    #expect(GitClient.ignorePattern(for: ".env", rule: .fileExtension) == nil)
    #expect(GitClient.ignorePattern(for: "line\nbreak", rule: .path) == nil)
}

@Test func ignoringUntrackedFilesAddsOneLiteralRuleInTheChosenFile() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "tracked\n".write(to: root.appendingPathComponent("tracked.log"), atomically: true, encoding: .utf8)
    try "keep".write(to: root.appendingPathComponent(".gitignore"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    for name in ["*.txt", "a.txt", "debug.log", "private.key"] {
        try "x".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }
    let untracked = { try git.loadStatus(in: root).filter { $0.kind == .untracked }.map(\.path).sorted() }

    try git.ignore(path: "*.txt", rule: .path, scope: .shared, in: root)
    try git.ignore(path: "*.txt", rule: .path, scope: .shared, in: root)
    #expect(try untracked() == ["a.txt", "debug.log", "private.key"])
    #expect(try String(contentsOf: root.appendingPathComponent(".gitignore"), encoding: .utf8) == "keep\n/\\*.txt\n")

    try git.ignore(path: "debug.log", rule: .fileExtension, scope: .shared, in: root)
    #expect(try untracked() == ["a.txt", "private.key"])

    try git.ignore(path: "private.key", rule: .path, scope: .local, in: root)
    #expect(try untracked() == ["a.txt"])
    #expect(try !String(contentsOf: root.appendingPathComponent(".gitignore"), encoding: .utf8).contains("private"))
    #expect(try String(contentsOf: root.appendingPathComponent(".git/info/exclude"), encoding: .utf8).hasSuffix("/private.key\n"))

    // Tracked files are not ignored by rules, so they are refused rather than silently kept.
    #expect(throws: (any Error).self) { try git.ignore(path: "tracked.log", rule: .path, scope: .shared, in: root) }
}

@Test func commitSearchCoversAllBranchesLiterally() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    #expect(try git.searchCommits("anything", in: .message, in: root).isEmpty)
    try git.setIdentity(name: "Ada Lovelace", email: "ada@example.invalid", in: root)
    try "let total = 1\n".write(to: root.appendingPathComponent("code.swift"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add total (v1.0)", in: root)
    try git.createBranch(named: "feature", in: root)
    try git.setIdentity(name: "Grace Hopper", email: "grace@example.invalid", in: root)
    try "let total = 1\nlet average = 2\n".write(to: root.appendingPathComponent("code.swift"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Compute AVERAGE on a side branch", in: root)
    let side = try #require(git.loadSnapshot(at: root).headHash)
    try git.checkout(branch: "main", in: root)

    // Branches other than the checkout are searched, and regex characters are literal.
    #expect(try git.searchCommits("average", in: .message, in: root).map(\.hash) == [side])
    #expect(try git.searchCommits("(v1.0)", in: .message, in: root).map(\.subject) == ["Add total (v1.0)"])
    #expect(try git.searchCommits(".*", in: .message, in: root).isEmpty)
    #expect(try git.searchCommits("grace", in: .author, in: root).map(\.hash) == [side])
    #expect(try git.searchCommits("let average", in: .change, in: root).map(\.hash) == [side])
    #expect(try git.searchCommits(String(side.prefix(8)), in: .author, in: root).first?.hash == side)
    #expect(try git.searchCommits("   ", in: .message, in: root).isEmpty)
}

private func makeRebaseFixture() throws -> (GitClient, URL, [String]) {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    var hashes: [String] = []
    for name in ["a", "b", "c", "d", "e"] {
        try "\(name)\n".write(to: root.appendingPathComponent("\(name).txt"), atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Add \(name)\n\nBody of \(name).", in: root)
        hashes.append(try #require(git.loadSnapshot(at: root).headHash))
    }
    return (git, root, hashes)
}

@Test func interactiveRebaseReordersRewordsSquashesFixesAndDrops() throws {
    let (git, root, hashes) = try makeRebaseFixture()
    defer { try? FileManager.default.removeItem(at: root) }
    let plan = try git.interactiveRebasePlan(from: hashes[1], in: root)
    #expect(plan.commits.map(\.hash) == Array(hashes[1...]))
    #expect(plan.base == hashes[0])
    #expect(plan.publishedCommits.isEmpty)
    let byHash = Dictionary(uniqueKeysWithValues: plan.commits.map { ($0.hash, $0) })
    let steps = [
        GitRebaseStep(commit: byHash[hashes[3]]!, action: .reword("Add d first\n\n#42 keeps its hash line")),
        GitRebaseStep(commit: byHash[hashes[1]]!, action: .squash),
        GitRebaseStep(commit: byHash[hashes[2]]!, action: .fixup),
        GitRebaseStep(commit: byHash[hashes[4]]!, action: .drop),
    ]
    let snapshot = try git.loadSnapshot(at: root)
    try git.interactiveRebase(steps, plan: plan, expectedBranch: snapshot.currentBranch, expectedHead: hashes[4], in: root)

    let after = try git.loadSnapshot(at: root)
    #expect(after.operation == nil)
    #expect(after.commits.map(\.subject) == ["Add d first", "Add a"])
    #expect(try git.commitMessage(hash: "HEAD", in: root).trimmingCharacters(in: .whitespacesAndNewlines)
        == "Add d first\n\n#42 keeps its hash line\n\nAdd b\n\nBody of b.")
    for name in ["a", "b", "c", "d"] { #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent("\(name).txt").path)) }
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("e.txt").path))
    #expect(after.status.isEmpty)
}

@Test func interactiveRebaseFromRootAndRefusals() throws {
    let (git, root, hashes) = try makeRebaseFixture()
    defer { try? FileManager.default.removeItem(at: root) }
    let branch = try git.loadSnapshot(at: root).currentBranch
    let plan = try git.interactiveRebasePlan(from: hashes[0], in: root)
    #expect(plan.base == nil && plan.commits.count == 5)
    var steps = plan.commits.map { GitRebaseStep(commit: $0) }

    steps[0].action = .squash
    #expect(throws: (any Error).self) { try git.interactiveRebase(steps, plan: plan, expectedBranch: branch, expectedHead: hashes[4], in: root) }
    steps[0].action = .pick
    #expect(throws: (any Error).self) { try git.interactiveRebase(Array(steps.dropLast()), plan: plan, expectedBranch: branch, expectedHead: hashes[4], in: root) }
    #expect(throws: (any Error).self) { try git.interactiveRebase(steps, plan: plan, expectedBranch: branch, expectedHead: hashes[3], in: root) }
    try "dirty\n".write(to: root.appendingPathComponent("a.txt"), atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.interactiveRebase(steps, plan: plan, expectedBranch: branch, expectedHead: hashes[4], in: root) }
    try runGit(["checkout", "--quiet", "--", "a.txt"], in: root)
    #expect(try git.loadSnapshot(at: root).headHash == hashes[4])

    steps[1].action = .fixup
    try git.interactiveRebase(steps, plan: plan, expectedBranch: branch, expectedHead: hashes[4], in: root)
    #expect(try git.loadSnapshot(at: root).commits.map(\.subject) == ["Add e", "Add d", "Add c", "Add a"])
}

@Test func interactiveRebaseRejectsMergesAndStopsForConflicts() throws {
    let (git, root, hashes) = try makeRebaseFixture()
    defer { try? FileManager.default.removeItem(at: root) }
    let branch = try git.loadSnapshot(at: root).currentBranch
    // Reordering two edits of the same line conflicts; the rebase stops for review.
    for text in ["one\n", "two\n"] {
        try text.write(to: root.appendingPathComponent("a.txt"), atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Set a to \(text)", in: root)
    }
    let head = try #require(git.loadSnapshot(at: root).headHash)
    let plan = try git.interactiveRebasePlan(from: "HEAD~1", in: root)
    let reversed = plan.commits.reversed().map { GitRebaseStep(commit: $0) }
    #expect(throws: (any Error).self) { try git.interactiveRebase(reversed, plan: plan, expectedBranch: branch, expectedHead: head, in: root) }
    #expect(try git.currentOperation(in: root) == .rebase)
    try git.abortOperation(.rebase, in: root)
    #expect(try git.loadSnapshot(at: root).headHash == head)

    try git.createBranch(named: "side", in: root)
    try "side\n".write(to: root.appendingPathComponent("side.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Side", in: root)
    try git.checkout(branch: branch, in: root)
    try runGit(["merge", "--quiet", "--no-ff", "--no-edit", "side"], in: root)
    #expect(throws: (any Error).self) { try git.interactiveRebasePlan(from: hashes[3], in: root) }
}

@Test func compareListsFilesBetweenCommitsAndWorkingFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    for name in ["keep.txt", "edit.txt", "gone.txt"] { try "one\n".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8) }
    try git.stageAll(in: root)
    try git.commit(message: "One", in: root)
    let first = try #require(git.loadSnapshot(at: root).headHash)
    try "two\n".write(to: root.appendingPathComponent("edit.txt"), atomically: true, encoding: .utf8)
    try FileManager.default.removeItem(at: root.appendingPathComponent("gone.txt"))
    try "new\n".write(to: root.appendingPathComponent("*.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Two", in: root)
    let second = try #require(git.loadSnapshot(at: root).headHash)
    try "three\n".write(to: root.appendingPathComponent("keep.txt"), atomically: true, encoding: .utf8)

    let between = try git.compareFiles(from: first, to: second, in: root)
    #expect(between.map(\.path) == ["*.txt", "edit.txt", "gone.txt"])
    #expect(between.map(\.status) == ["A", "M", "D"])
    #expect(try git.compareFiles(from: first, to: nil, in: root).map(\.path) == ["*.txt", "edit.txt", "gone.txt", "keep.txt"])
    let patch = try git.compareFileDiff(from: first, to: second, path: "*.txt", in: root)
    #expect(patch.contains("+new") && !patch.contains("edit.txt"))
    #expect(try git.compareFileDiff(from: second, to: nil, path: "keep.txt", in: root).contains("+three"))
    #expect(throws: (any Error).self) { try git.compareFiles(from: "--output=/tmp/x", to: nil, in: root) }
}

@Test func amendingIncludesStagedChangesOnlyForTheCapturedCheckout() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("checkout")
    let server = base.appendingPathComponent("server.git")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    try runGit(["init", "--quiet", "--bare", server.path], in: base)
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "First", in: root)
    try "two\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Second", in: root)
    let snapshot = try git.loadSnapshot(at: root)
    let head = try #require(snapshot.headHash)
    #expect(try !git.isPublished(head, in: root))

    try "extra\n".write(to: root.appendingPathComponent("extra.txt"), atomically: true, encoding: .utf8)
    try git.stage(path: "extra.txt", in: root)
    try "unstaged\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.amendCommit(message: "   ", expectedBranch: snapshot.currentBranch, expectedHead: head, in: root) }
    #expect(throws: (any Error).self) { try git.amendCommit(message: "Stale", expectedBranch: snapshot.currentBranch, expectedHead: snapshot.commits[1].hash, in: root) }
    try git.amendCommit(message: "Second, with extra", expectedBranch: snapshot.currentBranch, expectedHead: head, in: root)

    let after = try git.loadSnapshot(at: root)
    #expect(after.commits.map(\.subject) == ["Second, with extra", "First"])
    #expect(try git.commitFiles(hash: "HEAD", in: root).sorted() == ["extra.txt", "file.txt"])
    #expect(after.status.map(\.path) == ["file.txt"])

    try git.addRemote(name: "origin", address: server.path, in: root)
    try runGit(["push", "--quiet", "origin", "main"], in: root)
    try git.fetch(in: root)
    #expect(try git.isPublished(try #require(after.headHash), in: root))
}

@Test func reflogFindsCommitsLeftBehindByAResetAndRecoversThem() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    #expect(try git.reflog(in: root).isEmpty)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    for message in ["One", "Two", "Three"] {
        try "\(message)\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: message, in: root)
    }
    let lost = try #require(git.loadSnapshot(at: root).headHash)
    try runGit(["reset", "--quiet", "--hard", "HEAD~2"], in: root)
    #expect(!(try git.loadSnapshot(at: root).commits.contains { $0.hash == lost }))

    let entries = try git.reflog(in: root)
    #expect(entries.first?.action.hasPrefix("reset:") == true)
    let found = try #require(entries.first { $0.hash == lost })
    // Resetting two commits back leaves "Two" and "Three" on no branch.
    let unreachable = try git.unreachableCommits(entries.map(\.hash), in: root)
    #expect(Set(entries.filter { unreachable.contains($0.hash) }.map(\.subject)) == ["Two", "Three"])
    #expect(found.subject == "Three" && found.selector == "HEAD@{1}" && found.date != nil)
    try git.createBranch(named: "recovered", startingAt: found.hash, in: root)
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "recovered" }?.tip == lost)
}

@Test func stashingSelectedFilesLeavesEveryOtherFileAlone() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    for name in ["*.txt", "a.txt", "staged.txt"] { try "one\n".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8) }
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    for name in ["*.txt", "a.txt"] { try "two\n".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8) }
    try "staged\n".write(to: root.appendingPathComponent("staged.txt"), atomically: true, encoding: .utf8)
    try git.stage(path: "staged.txt", in: root)
    try "new\n".write(to: root.appendingPathComponent("new*.md"), atomically: true, encoding: .utf8)
    try "other\n".write(to: root.appendingPathComponent("other.md"), atomically: true, encoding: .utf8)

    try git.saveStash(paths: ["*.txt", "staged.txt", "new*.md"], message: "Selected", in: root)
    let status = try git.loadStatus(in: root)
    #expect(status.map(\.path).sorted() == ["a.txt", "other.md"])
    #expect(try git.listStashes(in: root).first?.message.contains("Selected") == true)
    #expect(try git.stashFiles(hash: "refs/stash", in: root) == ["*.txt", "new*.md", "staged.txt"])
    #expect(throws: (any Error).self) { try git.saveStash(paths: ["missing.txt"], message: "", in: root) }
    #expect(throws: (any Error).self) { try git.saveStash(paths: [], message: "", in: root) }
}

@Test func linkedWorktreesAreRemovedOnlyWhenCleanAndNotCurrent() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("main")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    for name in ["clean", "dirty", "gone"] {
        try runGit(["branch", name], in: root)
        try git.createWorktree(branch: name, at: base.appendingPathComponent(name), in: root)
    }
    let paths = try git.loadSnapshot(at: root).worktrees.map(\.path)
    let path = { (name: String) in try #require(paths.first { $0.hasSuffix("/" + name) }) }
    try "edit\n".write(to: URL(fileURLWithPath: try path("dirty")).appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)

    #expect(throws: (any Error).self) { try git.removeWorktree(at: paths[0], in: root) }
    #expect(throws: (any Error).self) { try git.removeWorktree(at: try path("clean"), in: URL(fileURLWithPath: try path("clean"))) }
    #expect(throws: (any Error).self) { try git.removeWorktree(at: try path("dirty"), in: root) }
    #expect(FileManager.default.fileExists(atPath: try path("dirty") + "/file.txt"))
    try git.removeWorktree(at: try path("clean"), in: root)
    #expect(!FileManager.default.fileExists(atPath: try path("clean")))

    try FileManager.default.removeItem(atPath: try path("gone"))
    #expect(try git.loadSnapshot(at: root).worktrees.contains { $0.path == (try? path("gone")) && $0.isPrunable })
    try git.pruneWorktrees(in: root)
    #expect(try git.loadSnapshot(at: root).worktrees.map(\.path) == [paths[0], try path("dirty")])
}

@Test func commitSignaturesAreVerifiedLocally() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let root = base.appendingPathComponent("repo")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "one\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Unsigned", in: root)
    #expect(try git.signature(of: "HEAD", in: root) == nil)

    let key = base.appendingPathComponent("key")
    let keygen = Process()
    keygen.executableURL = URL(fileURLWithPath: "/usr/bin/ssh-keygen")
    keygen.arguments = ["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f", key.path]
    try keygen.run()
    keygen.waitUntilExit()
    guard keygen.terminationStatus == 0 else { return }
    let publicKey = try String(contentsOf: key.appendingPathExtension("pub"), encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    try runGit(["config", "gpg.format", "ssh"], in: root)
    try runGit(["config", "user.signingkey", key.path], in: root)
    try "two\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try runGit(["commit", "--quiet", "-S", "-m", "Signed"], in: root)

    // Without an allowed-signers list, the signature cannot be checked here.
    let unchecked = try #require(try git.signature(of: "HEAD", in: root))
    #expect(unchecked.status != .verified && unchecked.status != .bad)
    // Strict verification needs a Git configuration that lets signatures be checked here;
    // an invalid global signing setting leaves every signature unverifiable, as reported.
    guard unchecked.problem == nil else { return }

    let signers = base.appendingPathComponent("allowed_signers")
    try "test@example.invalid \(publicKey)\n".write(to: signers, atomically: true, encoding: .utf8)
    try runGit(["config", "gpg.ssh.allowedSignersFile", signers.path], in: root)
    let verified = try #require(try git.signature(of: "HEAD", in: root))
    #expect(verified.status == .verified)
    #expect(verified.signer == "test@example.invalid")
    #expect(!verified.key.isEmpty)
}

@Test func diffsCanIgnoreWhitespaceOnlyChanges() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "a\nb\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try "  a\nB\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    let full = GitDiffLine.parse(try git.diff(path: "file.txt", staged: false, in: root)).filter { $0.kind == .addition }
    let quiet = GitDiffLine.parse(try git.diff(path: "file.txt", staged: false, ignoreWhitespace: true, in: root)).filter { $0.kind == .addition }
    #expect(full.map(\.text) == ["+  a", "+B"])
    #expect(quiet.map(\.text) == ["+B"])
    try git.stageAll(in: root)
    try git.commit(message: "Indent", in: root)
    #expect(GitDiffLine.parse(try git.commitFileDiff(hash: "HEAD", path: "file.txt", ignoreWhitespace: true, in: root)).filter { $0.kind == .addition }.map(\.text) == ["+B"])
}

@Test func fileVersionsAreReadAsBytesFromCommitsIndexAndDisk() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let first = Data([0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF])
    let staged = Data([0x00, 0x01])
    let working = Data([0xFE])
    try first.write(to: root.appendingPathComponent("image:1.png"))
    try git.stageAll(in: root)
    try git.commit(message: "Add", in: root)
    try staged.write(to: root.appendingPathComponent("image:1.png"))
    try git.stage(path: "image:1.png", in: root)
    try working.write(to: root.appendingPathComponent("image:1.png"))

    #expect(try git.fileData(path: "image:1.png", at: .revision("HEAD"), in: root) == first)
    #expect(try git.fileData(path: "image:1.png", at: .index, in: root) == staged)
    #expect(try git.fileData(path: "image:1.png", at: .workingFile, in: root) == working)
    #expect(try git.fileData(path: "image:1.png", at: .revision("HEAD^1"), in: root) == nil)
    #expect(try git.fileData(path: "missing.png", at: .revision("HEAD"), in: root) == nil)
    #expect(try git.fileData(path: "missing.png", at: .workingFile, in: root) == nil)
    #expect(throws: (any Error).self) { try git.fileData(path: "image:1.png", at: .revision("HEAD"), limit: 2, in: root) }
}

@Test func submodulesAreListedFromTheIndexAndUpdatedOneAtATime() throws {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let library = base.appendingPathComponent("library")
    let app = base.appendingPathComponent("app")
    let clone = base.appendingPathComponent("clone")
    try FileManager.default.createDirectory(at: library, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: app, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: base) }
    let git = GitClient()
    for repository in [library, app] {
        try git.initialize(at: repository)
        try git.setIdentity(name: "Test", email: "test@example.invalid", in: repository)
    }
    try "one\n".write(to: library.appendingPathComponent("lib.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: library)
    try git.commit(message: "Library one", in: library)
    let recorded = try #require(git.loadSnapshot(at: library).headHash)
    try runGit(["-c", "protocol.file.allow=always", "submodule", "add", "--quiet", library.path, "vendor/lib x"], in: app)
    try git.commit(message: "Add library", in: app)
    #expect(try git.submodules(in: app) == [GitSubmodule(path: "vendor/lib x", recordedCommit: recorded, state: .upToDate, hasLocalChanges: false)])

    try runGit(["clone", "--quiet", app.path, clone.path], in: base)
    #expect(try git.submodules(in: clone).first?.state == .uninitialized)
    // Git refuses file-transport submodule clones by default; the fixture opts in for setup only.
    try runGit(["-c", "protocol.file.allow=always", "submodule", "update", "--quiet", "--init"], in: clone)
    #expect(try git.submodules(in: clone).first?.state == .upToDate)

    let inside = clone.appendingPathComponent("vendor/lib x")
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: inside)
    try "two\n".write(to: inside.appendingPathComponent("lib.txt"), atomically: true, encoding: .utf8)
    #expect(try git.submodules(in: clone).first?.hasLocalChanges == true)
    try git.stageAll(in: inside)
    try git.commit(message: "Local library change", in: inside)
    let local = try #require(git.loadSnapshot(at: inside).headHash)
    #expect(try git.submodules(in: clone).first?.state == .differentCommit(local))
    #expect(throws: (any Error).self) { try git.updateSubmodule("vendor/other", in: clone) }
    try git.updateSubmodule("vendor/lib x", in: clone)
    #expect(try git.submodules(in: clone).first?.state == .upToDate)
}

@Test func discardsCanBeUndoneExactlyUntilThePathChangesAgain() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    for name in ["*.txt", "deleted.txt", "tool.sh"] { try "base\n".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8) }
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)

    try "staged\n".write(to: root.appendingPathComponent("*.txt"), atomically: true, encoding: .utf8)
    try git.stage(path: "*.txt", in: root)
    try "unstaged\r\n".write(to: root.appendingPathComponent("*.txt"), atomically: true, encoding: .utf8)
    try FileManager.default.removeItem(at: root.appendingPathComponent("deleted.txt"))
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: root.appendingPathComponent("tool.sh").path)
    try "new\n".write(to: root.appendingPathComponent("added.txt"), atomically: true, encoding: .utf8)
    try git.stage(path: "added.txt", in: root)
    try "untracked\n".write(to: root.appendingPathComponent("untracked.txt"), atomically: true, encoding: .utf8)
    let before = try git.loadStatus(in: root)

    var undos: [GitDiscardUndo] = []
    for entry in before { undos.append(try #require(try git.discardKeepingUndo(entry, in: root))) }
    #expect(try git.loadStatus(in: root).isEmpty)
    for undo in undos.reversed() { try git.undoDiscard(undo, in: root) }
    #expect(try git.loadStatus(in: root) == before)
    #expect(try Data(contentsOf: root.appendingPathComponent("*.txt")) == Data("unstaged\r\n".utf8))
    #expect(try git.diff(path: "*.txt", staged: true, in: root).contains("+staged"))
    #expect((try FileManager.default.attributesOfItem(atPath: root.appendingPathComponent("tool.sh").path)[.posixPermissions] as? Int) == 0o755)

    // Newer work on the path is never overwritten by an old undo.
    let entry = try #require(try git.loadStatus(in: root).first { $0.path == "untracked.txt" })
    let undo = try #require(try git.discardKeepingUndo(entry, in: root))
    try "newer\n".write(to: root.appendingPathComponent("untracked.txt"), atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try git.undoDiscard(undo, in: root) }
    #expect(try String(contentsOf: root.appendingPathComponent("untracked.txt"), encoding: .utf8) == "newer\n")
}

@Test func severalCommitsAreCherryPickedOldestFirst() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "base\n".write(to: root.appendingPathComponent("base.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", in: root)
    var picked: [String] = []
    for (index, name) in ["one", "two", "skip", "three"].enumerated() {
        try "\(name)\n".write(to: root.appendingPathComponent("\(name).txt"), atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        let date = "2026-01-0\(index + 1)T00:00:00"
        try runGit(["commit", "--quiet", "-m", name, "--date", date], in: root, environment: ["GIT_COMMITTER_DATE": date])
        if name != "skip" { picked.append(try #require(git.loadSnapshot(at: root).headHash)) }
    }
    try git.checkout(branch: "main", in: root)
    let head = try git.loadSnapshot(at: root).headHash

    #expect(throws: (any Error).self) { try git.cherryPick(picked, expectedHead: "0000000", expectedBranch: "main", in: root) }
    try git.cherryPick(picked.reversed(), expectedHead: head, expectedBranch: "main", in: root)
    #expect(try runGitOutput(["log", "--format=%s", "-4", "main"], in: root).split(separator: "\n") == ["three", "two", "one", "Base"])
    #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("skip.txt").path))
}

@Test func applyingAnIdentityWritesOnlyRepositorySettings() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.applyIdentity(name: "Work Me", email: "me@work.invalid", signingKey: nil, in: root)
    #expect(git.identity(in: root) == ("Work Me", "me@work.invalid"))
    #expect((try? runGitOutput(["config", "--local", "commit.gpgsign"], in: root)) == nil)
    try git.applyIdentity(name: "Open Me", email: "me@home.invalid", signingKey: " ~/.ssh/id_ed25519.pub ", in: root)
    #expect(git.identity(in: root) == ("Open Me", "me@home.invalid"))
    #expect(try runGitOutput(["config", "--local", "user.signingkey"], in: root) == "~/.ssh/id_ed25519.pub\n")
    #expect(try runGitOutput(["config", "--local", "commit.gpgsign"], in: root) == "true\n")
    #expect(throws: (any Error).self) { try git.applyIdentity(name: " ", email: "x@y.invalid", signingKey: nil, in: root) }
}

@Test func gitFlowStartsAndFinishesFeaturesAndReleases() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "base\n".write(to: root.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    #expect(git.gitFlowConfiguration(in: root) == nil)
    try git.initializeGitFlow(GitFlowConfiguration(versionTagPrefix: "v"), in: root)
    #expect(git.gitFlowConfiguration(in: root)?.developBranch == "develop")
    let head = { try git.loadSnapshot(at: root) }

    var snapshot = try head()
    try git.startGitFlow(.feature, name: "login", expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash, in: root)
    #expect(try head().currentBranch == "feature/login")
    try "login\n".write(to: root.appendingPathComponent("login.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Add login", in: root)
    snapshot = try head()
    try git.finishGitFlow(expectedBranch: "feature/login", expectedHead: snapshot.headHash, in: root)
    snapshot = try head()
    #expect(snapshot.currentBranch == "develop")
    #expect(!snapshot.branches.contains { $0.name == "feature/login" })
    #expect(snapshot.commits.first?.parents.count == 2)

    try git.startGitFlow(.release, name: "1.0", expectedBranch: "develop", expectedHead: snapshot.headHash, in: root)
    try "1.0\n".write(to: root.appendingPathComponent("VERSION"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Bump version", in: root)
    // Develop moves on with a conflicting change, so finishing stops at the second merge.
    try git.checkout(branch: "develop", in: root)
    try "develop\n".write(to: root.appendingPathComponent("VERSION"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Develop version", in: root)
    try git.checkout(branch: "release/1.0", in: root)
    snapshot = try head()
    #expect(throws: (any Error).self) { try git.finishGitFlow(expectedBranch: "release/1.0", expectedHead: snapshot.headHash, in: root) }
    #expect(try git.currentOperation(in: root) == .merge)
    #expect(try runGitOutput(["tag", "--list", "v1.0"], in: root) == "v1.0\n")
    try "1.0\n".write(to: root.appendingPathComponent("VERSION"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.continueOperation(.merge, in: root)
    try git.checkout(branch: "release/1.0", in: root)
    snapshot = try head()
    try git.finishGitFlow(expectedBranch: "release/1.0", expectedHead: snapshot.headHash, in: root)
    snapshot = try head()
    #expect(snapshot.currentBranch == "develop")
    #expect(!snapshot.branches.contains { $0.name == "release/1.0" })
    #expect(try runGitOutput(["merge-base", "--is-ancestor", "v1.0", "main"], in: root).isEmpty)
    #expect(throws: (any Error).self) { try git.finishGitFlow(expectedBranch: "develop", expectedHead: snapshot.headHash, in: root) }
}

@Test func lfsPatternsAndFilesAreReadFromGitAttributes() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try """
    *.txt text
    *.psd filter=lfs diff=lfs merge=lfs -text
    "art files/*.png" filter=lfs diff=lfs merge=lfs -text
    """.write(to: root.appendingPathComponent(".gitattributes"), atomically: true, encoding: .utf8)
    try "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 3\n".write(to: root.appendingPathComponent("pointer.psd"), atomically: true, encoding: .utf8)
    try "notes\n".write(to: root.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)
    try FileManager.default.createDirectory(at: root.appendingPathComponent("art files"), withIntermediateDirectories: true)
    try Data([0x89, 0x50]).write(to: root.appendingPathComponent("art files/logo.png"))
    // Stage without filters: git-lfs may be missing, and the pointer file is the committed form.
    try runGit(["-c", "filter.lfs.clean=cat", "-c", "filter.lfs.smudge=cat", "add", "--all"], in: root)
    try runGit(["-c", "filter.lfs.clean=cat", "-c", "filter.lfs.smudge=cat", "commit", "--quiet", "-m", "Assets"], in: root)

    let status = try git.lfsStatus(in: root)
    #expect(status.patterns == ["*.psd", "art files/*.png"])
    #expect(status.files == [GitLFSFile(path: "art files/logo.png", isPointerOnly: false), GitLFSFile(path: "pointer.psd", isPointerOnly: true)])

    if status.version == nil {
        #expect(throws: (any Error).self) { try git.trackLFS("*.mov", in: root) }
    } else {
        try git.trackLFS("my clips/*.mov", in: root)
        #expect(try git.lfsStatus(in: root).patterns.last == "my clips/*.mov")
    }
    try git.untrackLFS("*.psd", in: root)
    let attributes = try String(contentsOf: root.appendingPathComponent(".gitattributes"), encoding: .utf8)
    #expect(attributes.hasPrefix("*.txt text\n\"art files/*.png\" filter=lfs"))
    #expect(!attributes.contains("*.psd"))
}
