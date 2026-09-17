import Foundation
@testable import NiceGitCore
import Testing

@Test func adjacentReplacementKeepsLineOrder() throws {
    try reviewFixture { git, root in
        let file = root.appendingPathComponent("adjacent.txt")
        try "one\ntwo\n".write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Base", in: root)
        try "ONE\nTWO\n".write(to: file, atomically: true, encoding: .utf8)
        let review = try git.fileReview(path: "adjacent.txt", staged: false, in: root)
        let chosen = Set(review.lines.indices.filter { ["-one", "+ONE"].contains(review.lines[$0].text) })
        try git.stageLines(chosen, from: review, in: root)
        #expect(try git.run(["show", ":adjacent.txt"], in: root) == "ONE\ntwo\n")
        let staged = try git.fileReview(path: "adjacent.txt", staged: true, in: root)
        let undo = Set(staged.lines.indices.filter { ["-one", "+ONE"].contains(staged.lines[$0].text) })
        try git.stageLines(undo, from: staged, in: root)
        #expect(try git.run(["show", ":adjacent.txt"], in: root) == "one\ntwo\n")
    }
}

@Test func separateHunksStageIndependently() throws {
    let noNewline = GitDiffLine.parse("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n")
    #expect(GitDiffHunk.grouped(noNewline).count == 1)
    try reviewFixture { git, root in
        let file = root.appendingPathComponent("hunks.txt")
        let original = (1...30).map { "line \($0)" }.joined(separator: "\n") + "\n"
        try original.write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Base", in: root)
        let modified = original.replacingOccurrences(of: "line 3\n", with: "changed 3\n").replacingOccurrences(of: "line 25\n", with: "changed 25\n")
        try modified.write(to: file, atomically: true, encoding: .utf8)
        let review = try git.fileReview(path: "hunks.txt", staged: false, in: root)
        let hunks = GitDiffHunk.grouped(review.lines)
        #expect(hunks.count == 2)
        #expect(!hunks.flatMap(\.lineIndices).contains { review.lines[$0].text == " line 15" })
        try git.stageLines(try #require(hunks.first).changedIndices, from: review, in: root)
        #expect(try git.run(["show", ":hunks.txt"], in: root) == original.replacingOccurrences(of: "line 3\n", with: "changed 3\n"))
        let staged = try git.fileReview(path: "hunks.txt", staged: true, in: root)
        try git.stageLines(try #require(GitDiffHunk.grouped(staged.lines).first).changedIndices, from: staged, in: root)
        #expect(try git.run(["show", ":hunks.txt"], in: root) == original)
        #expect(try String(contentsOf: file, encoding: .utf8) == modified)
    }
}

private func reviewFixture(_ operation: (GitClient, URL) throws -> Void) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Review Test", email: "test@example.invalid", in: root)
    try operation(git, root)
}

@Test func partialStageAndUnstagePreserveUnselectedAndWorkingChanges() throws {
    try reviewFixture { git, root in
        let path = "file with spaces.txt"
        let file = root.appendingPathComponent(path)
        try "one\ntwo\nthree\n".write(to: file, atomically: true, encoding: .utf8)
        try git.stageAll(in: root)
        try git.commit(message: "Base", in: root)
        try "ONE\ntwo\nTHREE\n".write(to: file, atomically: true, encoding: .utf8)
        let review = try git.fileReview(path: path, staged: false, in: root)
        let chosen = Set(review.lines.indices.filter { ["-one", "+ONE"].contains(review.lines[$0].text) })
        try git.stageLines(chosen, from: review, in: root)
        #expect(try git.run(["show", ":" + path], in: root) == "ONE\ntwo\nthree\n")
        #expect(try String(contentsOf: file, encoding: .utf8) == "ONE\ntwo\nTHREE\n")
        #expect(throws: (any Error).self) { try git.stageLines(chosen, from: review, in: root) }
        let staged = try git.fileReview(path: path, staged: true, in: root)
        let addition = try #require(staged.lines.firstIndex { $0.text == "+ONE" })
        try git.stageLines([addition], from: staged, in: root)
        #expect(try git.run(["show", ":" + path], in: root) == "two\nthree\n")
        #expect(try String(contentsOf: file, encoding: .utf8) == "ONE\ntwo\nTHREE\n")
    }
}

@Test func partialStageNewFileAndUnstageDeletedFile() throws {
    try reviewFixture { git, root in
        let path = "new\tfile.txt"
        let file = root.appendingPathComponent(path)
        try "first\nsecond".write(to: file, atomically: true, encoding: .utf8)
        let review = try git.fileReview(path: path, staged: false, in: root)
        let chosen = try #require(review.lines.firstIndex { $0.text == "+second" })
        try git.stageLines([chosen], from: review, in: root)
        #expect(try git.run(["show", ":" + path], in: root) == "second")
        #expect(try String(contentsOf: file, encoding: .utf8) == "first\nsecond")
        try git.stageAll(in: root)
        try git.commit(message: "Base", in: root)
        try FileManager.default.removeItem(at: file)
        try git.stageAll(in: root)
        let deleted = try git.fileReview(path: path, staged: true, in: root)
        let restore = try #require(deleted.lines.firstIndex { $0.text == "-second" })
        try git.stageLines([restore], from: deleted, in: root)
        #expect(try git.run(["show", ":" + path], in: root) == "second")
        #expect(!FileManager.default.fileExists(atPath: file.path))
    }
}

@Test func fileEditorPreservesIndexAndRejectsExternalEditsAndSymlinks() throws {
    try reviewFixture { git, root in
        let file = root.appendingPathComponent("code.swift")
        try "let count = 1\n".write(to: file, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: file.path)
        try git.stageAll(in: root)
        let document = try git.editableFile(path: "code.swift", in: root)
        try git.saveFile(document, text: "let count = 2\n", in: root)
        #expect(try git.run(["show", ":code.swift"], in: root) == "let count = 1\n")
        #expect(try String(contentsOf: file, encoding: .utf8) == "let count = 2\n")
        #expect((try FileManager.default.attributesOfItem(atPath: file.path)[.posixPermissions] as? NSNumber)?.intValue == 0o755)
        #expect(throws: (any Error).self) { try git.saveFile(document, text: "stale", in: root) }
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("link"), withDestinationURL: file)
        #expect(throws: (any Error).self) { try git.editableFile(path: "link", in: root) }
        #expect(throws: (any Error).self) { try git.editableFile(path: ".git/config", in: root) }
        #expect(throws: (any Error).self) { try git.editableFile(path: "../outside", in: root) }
    }
}
