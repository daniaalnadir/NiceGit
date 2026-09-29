import Foundation

public struct GitSubmodule: Identifiable, Equatable, Sendable {
    public enum State: Equatable, Sendable {
        /// Registered but not checked out yet.
        case uninitialized
        /// Checked out at the commit the superproject records.
        case upToDate
        /// Checked out at a different commit than the superproject records.
        case differentCommit(String)
    }
    public let path: String
    /// The commit the superproject records for this submodule.
    public let recordedCommit: String
    public let state: State
    /// Uncommitted changes inside the submodule's own working tree.
    public let hasLocalChanges: Bool
    public var id: String { path }
    public init(path: String, recordedCommit: String, state: State, hasLocalChanges: Bool) {
        self.path = path; self.recordedCommit = recordedCommit; self.state = state; self.hasLocalChanges = hasLocalChanges
    }
}

extension GitClient {
    public func submodules(in url: URL) throws -> [GitSubmodule] {
        // The index lists each submodule as a gitlink (mode 160000); unlike `submodule status`,
        // its NUL-separated output keeps paths with spaces or newlines unambiguous.
        let entries = try run(["ls-files", "-z", "--stage"], in: url).split(separator: "\0").compactMap { record -> (String, String)? in
            let parts = record.split(separator: "\t", maxSplits: 1)
            let fields = parts.first?.split(separator: " ") ?? []
            guard parts.count == 2, fields.count == 3, fields[0] == "160000", fields[2] == "0" else { return nil }
            return (String(parts[1]), String(fields[1]))
        }
        return entries.map { path, recorded -> GitSubmodule in
            let directory = url.appendingPathComponent(path)
            guard FileManager.default.fileExists(atPath: directory.appendingPathComponent(".git").path),
                  let head = try? run(["rev-parse", "--verify", "HEAD"], in: directory).trimmingCharacters(in: .whitespacesAndNewlines) else {
                return GitSubmodule(path: path, recordedCommit: recorded, state: .uninitialized, hasLocalChanges: false)
            }
            let dirty = !((try? run(["status", "--porcelain", "--ignore-submodules=none"], in: directory)) ?? "").isEmpty
            return GitSubmodule(path: path, recordedCommit: recorded, state: head == recorded ? .upToDate : .differentCommit(head), hasLocalChanges: dirty)
        }
    }

    /// Checks out the recorded commit in one submodule, initialising it first if needed. Git
    /// refuses when the submodule has uncommitted changes that the checkout would overwrite.
    public func updateSubmodule(_ path: String, in url: URL) throws {
        guard try submodules(in: url).contains(where: { $0.path == path }) else {
            throw GitClientError.commandFailed(command: "submodule update", message: "This submodule is no longer registered. Refresh the repository.")
        }
        try requireFinishedOperation(command: "submodule update", in: url)
        // submodule runs helper commands with their own pathspecs, so mark this one literal.
        try run(["submodule", "update", "--init", "--", ":(literal)" + path], in: url)
    }
}
