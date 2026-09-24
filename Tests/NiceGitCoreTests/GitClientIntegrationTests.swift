import Foundation
import NiceGitCore
import Testing

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
    for operation in [GitOperation.merge, .rebase] {
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
    for operation in [GitOperation.merge, .rebase] {
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

@Test func branchSwitchDoesNotOverwriteIgnoredLocalFile() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    try "generated.txt\n".write(to: root.appendingPathComponent(".gitignore"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Ignore generated file", in: root)
    try git.createBranch(named: "feature", in: root)
    let ignoredFile = root.appendingPathComponent("generated.txt")
    try "tracked feature\n".write(to: ignoredFile, atomically: true, encoding: .utf8)
    try runGit(["add", "--force", "generated.txt"], in: root)
    try git.commit(message: "Track generated file", in: root)
    try git.checkout(branch: "main", in: root)
    try "ignored local\n".write(to: ignoredFile, atomically: true, encoding: .utf8)
    #expect(try git.loadStatus(in: root).isEmpty)

    #expect(throws: (any Error).self) { try git.checkout(branch: "feature", in: root) }
    #expect(try git.loadSnapshot(at: root).currentBranch == "main")
    #expect(try String(contentsOf: ignoredFile, encoding: .utf8) == "ignored local\n")
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

    try git.push(expectedBranch: "main", expectedHead: main.headHash, in: root)
    #expect(try git.commitMessage(hash: "refs/heads/feature", in: remote).contains("Base"))
    #expect(throws: (any Error).self) { try runGit(["show-ref", "--verify", "refs/tags/unwanted"], in: remote) }
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
    #expect(try git.loadSnapshot(at: root).branches.first { $0.name == "selected-tip" }?.tip == base)
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
    try git.createWorktree(branch: "linked-feature", at: linked, in: main)
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

@Test func unstageBeforeFirstCommitPreservesFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try runGit(["init"], in: root)
    let file = root.appendingPathComponent("first.txt")
    try "keep me".write(to: file, atomically: true, encoding: .utf8)
    let git = GitClient()
    try git.stageAll(in: root)
    try git.unstage(path: "first.txt", in: root)
    #expect(try git.loadSnapshot(at: root).stagedCount == 0)
    try git.stageAll(in: root)
    try git.unstageAll(in: root)
    #expect(try git.loadSnapshot(at: root).stagedCount == 0)
    #expect(try String(contentsOf: file, encoding: .utf8) == "keep me")
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
    #expect(detached.commits.first?.subject == "Detached commit")
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

private func runGit(_ arguments: [String], in directory: URL) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["git"] + arguments
    process.currentDirectoryURL = directory
    process.environment = ["PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"]

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
