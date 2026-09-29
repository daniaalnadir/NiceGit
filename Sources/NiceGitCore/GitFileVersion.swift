import Foundation

/// Where to read one version of a file from.
public enum GitFileVersion: Equatable, Sendable {
    /// The file in a commit, named by any revision such as a hash or `abc123^1`.
    case revision(String)
    /// The staged version.
    case index
    /// The file on disk.
    case workingFile
}

extension GitClient {
    /// The bytes of `path` in `version`, or nil when that version has no such file. Files larger
    /// than `limit` are refused rather than loaded.
    public func fileData(path: String, at version: GitFileVersion, limit: Int = 50_000_000, in url: URL) throws -> Data? {
        let tooLarge = GitClientError.commandFailed(command: "show file", message: "This file is too large to preview.")
        let object: String
        switch version {
        case .workingFile:
            let file = url.appendingPathComponent(path)
            guard let attributes = try? FileManager.default.attributesOfItem(atPath: file.path),
                  attributes[.type] as? FileAttributeType == .typeRegular else { return nil }
            guard (attributes[.size] as? Int ?? 0) <= limit else { throw tooLarge }
            return try Data(contentsOf: file)
        case .index:
            object = ":0:" + path
        case let .revision(revision):
            guard let commit = try? run(["rev-parse", "--verify", "--end-of-options", revision + "^{commit}"], in: url)
                .trimmingCharacters(in: .whitespacesAndNewlines) else { return nil }
            object = commit + ":" + path
        }
        guard (try? run(["cat-file", "-e", object], in: url)) != nil,
              let type = try? run(["cat-file", "-t", object], in: url).trimmingCharacters(in: .whitespacesAndNewlines), type == "blob" else { return nil }
        guard let size = Int(try run(["cat-file", "-s", object], in: url).trimmingCharacters(in: .whitespacesAndNewlines)), size <= limit else { throw tooLarge }
        return try runData(["cat-file", "blob", object], in: url)
    }
}
