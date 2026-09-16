import Foundation

public struct GitConflictDocument: Sendable {
    public let path: String
    public let content: String
    public let originalData: Data
    public let base: String?
    public let current: String?
    public let incoming: String?
}

extension GitClient {
    public func loadConflict(path: String, in repository: URL) throws -> GitConflictDocument {
        let file = try conflictFile(path: path, in: repository)
        let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
        guard (attributes[.size] as? NSNumber)?.intValue ?? 0 <= 2_000_000 else {
            throw conflictError("This file is too large for the built-in editor.")
        }
        let data = try Data(contentsOf: file)
        guard !data.contains(0), let content = String(data: data, encoding: .utf8) else {
            throw conflictError("This file is not UTF-8 text. Choose a whole-file resolution or use an external editor.")
        }
        return GitConflictDocument(path: path, content: content, originalData: data,
                                   base: conflictVersion(path: path, stage: 1, in: repository),
                                   current: conflictVersion(path: path, stage: 2, in: repository),
                                   incoming: conflictVersion(path: path, stage: 3, in: repository))
    }

    public func resolveConflict(_ document: GitConflictDocument, content: String, in repository: URL) throws {
        let file = try conflictFile(path: document.path, in: repository)
        guard try Data(contentsOf: file) == document.originalData else {
            throw conflictError("The file changed outside this editor. Close and reopen it before saving.")
        }
        guard !content.split(separator: "\n").contains(where: {
            $0.hasPrefix("<<<<<<<") || $0.hasPrefix("=======") || $0.hasPrefix(">>>>>>>") || $0.hasPrefix("|||||||")
        }) else {
            throw conflictError("Remove the conflict markers before saving the resolution.")
        }
        let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
        try Data(content.utf8).write(to: file, options: .atomic)
        if let permissions = attributes[.posixPermissions] {
            try FileManager.default.setAttributes([.posixPermissions: permissions], ofItemAtPath: file.path)
        }
        try stage(path: document.path, in: repository)
    }

    private func conflictFile(path: String, in repository: URL) throws -> URL {
        let snapshot = try loadSnapshot(at: repository)
        guard snapshot.status.contains(where: { $0.path == path && $0.kind == .conflicted }) else {
            throw conflictError("This file no longer has an unresolved conflict.")
        }
        let root = URL(fileURLWithPath: snapshot.rootPath).resolvingSymlinksInPath()
        let file = root.appendingPathComponent(path)
        let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              file.resolvingSymlinksInPath().path.hasPrefix(root.path + "/") else {
            throw conflictError("Only regular files inside the repository can be edited here.")
        }
        return file
    }

    private func conflictError(_ message: String) -> GitClientError {
        .commandFailed(command: "resolve conflict", message: message)
    }
}
