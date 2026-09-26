import Foundation
@testable import NiceGitCore
import Testing

@Test(arguments: [GitOperation.cherryPick, .revert])
func pendingSequenceRemainsVisibleAfterManualConflictCommit(operation: GitOperation) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.setIdentity(name: "Test", email: "test@example.invalid", in: root)
    let file = root.appendingPathComponent("first.txt")
    try "base\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Base", in: root)
    try git.createBranch(named: "feature", in: root)
    try "feature\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "First", in: root)
    let first = try git.run(["rev-parse", "HEAD"], in: root).trimmingCharacters(in: .whitespacesAndNewlines)
    try "second\n".write(to: root.appendingPathComponent("second.txt"), atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Second", in: root)
    let second = try git.run(["rev-parse", "HEAD"], in: root).trimmingCharacters(in: .whitespacesAndNewlines)
    if operation == .cherryPick { try git.checkout(branch: "main", in: root) }
    try "diverged\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Diverged", in: root)
    #expect(throws: (any Error).self) { try git.run([operation.rawValue, first, second], in: root) }
    try "resolved\n".write(to: file, atomically: true, encoding: .utf8)
    try git.stageAll(in: root)
    try git.commit(message: "Resolve first step manually", in: root)
    #expect(try git.currentOperation(in: root) == operation)
    #expect(try git.loadSnapshot(at: root).operation == operation)
    let head = try git.run(["rev-parse", "HEAD"], in: root)
    #expect(throws: (any Error).self) {
        try git.checkout(branch: operation == .cherryPick ? "feature" : "main", in: root)
    }
    #expect(try git.run(["rev-parse", "HEAD"], in: root) == head)
    try git.continueOperation(operation, in: root)
    #expect(try git.currentOperation(in: root) == nil)
    #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent("second.txt").path) == (operation == .cherryPick))
}
